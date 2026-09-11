//! Capture-local logical admission. Retained charges are monotone; scratch is separate.
use super::{DevMapError, fail};
use std::collections::BTreeSet;
use std::io::{self, Read};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

pub(super) const MAX_FILE_BYTES: usize = 1024 * 1024;
pub(super) const SCRATCH_BYTES: usize = 8192;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_NODES: usize = 4096;
const MAX_PATH_BYTES: usize = 1024 * 1024;
const MAX_DIRECTORIES: usize = 4096;

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AdmissionTotals {
    pub(super) bytes: usize,
    pub(super) nodes: usize,
    pub(super) path_bytes: usize,
    pub(super) directories: usize,
}
#[derive(Default)]
struct AdmissionState {
    totals: AdmissionTotals,
    reserved: usize,
    cancelled: bool,
    directories: BTreeSet<std::path::PathBuf>,
}
#[cfg(test)]
type Observer<'h> = &'h (dyn Fn(usize, usize, usize, usize) + Sync);
pub(super) struct Admission<'h> {
    state: Mutex<AdmissionState>,
    #[cfg(test)]
    observer: Option<Observer<'h>>,
    lifetime: std::marker::PhantomData<&'h ()>,
}
pub(super) struct ChunkReservation<'a, 'h> {
    admission: &'a Admission<'h>,
    amount: usize,
    outstanding: bool,
}
impl<'h> Admission<'h> {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(AdmissionState::default()),
            #[cfg(test)]
            observer: None,
            lifetime: std::marker::PhantomData,
        }
    }
    #[cfg(test)]
    pub(super) fn with_observer(observer: Observer<'h>) -> Self {
        Self {
            state: Mutex::new(AdmissionState::default()),
            observer: Some(observer),
            lifetime: std::marker::PhantomData,
        }
    }
    // Immutable test instrumentation only: observer must not block or start work.
    fn admitted(&self, _state: &AdmissionState) {
        #[cfg(test)]
        if let Some(observer) = self.observer {
            observer(
                _state.totals.bytes,
                _state.totals.nodes,
                _state.totals.path_bytes,
                _state.totals.directories,
            );
        }
    }

    fn active(&self) -> Result<MutexGuard<'_, AdmissionState>, DevMapError> {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poison) => {
                poison.into_inner().cancelled = true;
                return Err(fail("origin proof admission poisoned"));
            }
        };
        if state.cancelled {
            return Err(fail("origin proof admission cancelled"));
        }
        Ok(state)
    }
    pub(super) fn charge_path(&self, path: &Path) -> Result<(), DevMapError> {
        let mut state = self.active()?;
        let bytes = state
            .totals
            .path_bytes
            .checked_add(path.as_os_str().as_encoded_bytes().len())
            .filter(|n| *n <= MAX_PATH_BYTES)
            .ok_or_else(|| fail("origin proof path byte limit"))?;
        state.totals.path_bytes = bytes;
        self.admitted(&state);
        Ok(())
    }
    pub(super) fn charge_node(&self) -> Result<(), DevMapError> {
        let mut state = self.active()?;
        let nodes = state
            .totals
            .nodes
            .checked_add(1)
            .filter(|n| *n <= MAX_NODES)
            .ok_or_else(|| fail("origin proof entry limit"))?;
        state.totals.nodes = nodes;
        self.admitted(&state);
        Ok(())
    }
    // Production callers supply keys already checked by safe canonicalization.
    // Each call separately charges its path, even when membership already exists.
    pub(super) fn admit_directory(&self, canonical: &Path) -> Result<(), DevMapError> {
        let mut state = self.active()?;
        if state.directories.contains(canonical) {
            return Ok(());
        }
        if state.directories.len() >= MAX_DIRECTORIES {
            return Err(fail("origin proof directory limit"));
        }
        state.directories.insert(canonical.to_owned());
        state.totals.directories = state.directories.len();
        self.admitted(&state);
        Ok(())
    }
    pub(super) fn reserve_chunk(
        &self,
        actual_len: usize,
    ) -> Result<ChunkReservation<'_, 'h>, DevMapError> {
        if actual_len == 0 || actual_len > SCRATCH_BYTES {
            return Err(fail("origin proof invalid chunk size"));
        }
        let mut state = self.active()?;
        let reserved = state
            .reserved
            .checked_add(actual_len)
            .ok_or_else(|| fail("origin proof byte limit"))?;
        state
            .totals
            .bytes
            .checked_add(reserved)
            .filter(|n| *n <= MAX_BYTES)
            .ok_or_else(|| fail("origin proof byte limit"))?;
        state.reserved = reserved;
        Ok(ChunkReservation {
            admission: self,
            amount: actual_len,
            outstanding: true,
        })
    }
    pub(super) fn cancel(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.cancelled = true;
    }
    pub(super) fn finish(&self) -> Result<AdmissionTotals, DevMapError> {
        let state = self.active()?;
        if state.reserved != 0 {
            return Err(fail("origin proof outstanding reservation"));
        }
        Ok(state.totals)
    }
    #[cfg(test)]
    fn snapshot(&self) -> Snapshot {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        Snapshot {
            totals: state.totals,
            reserved: state.reserved,
            cancelled: state.cancelled,
        }
    }
}
impl ChunkReservation<'_, '_> {
    pub(super) fn commit_before_append(mut self) -> Result<(), DevMapError> {
        let mut state = self.admission.active()?;
        let Some(reserved) = state.reserved.checked_sub(self.amount) else {
            state.cancelled = true;
            return Err(fail("origin proof reservation accounting inconsistent"));
        };
        let Some(bytes) = state
            .totals
            .bytes
            .checked_add(self.amount)
            .filter(|n| *n <= MAX_BYTES)
        else {
            state.cancelled = true;
            return Err(fail("origin proof reservation accounting inconsistent"));
        };
        state.reserved = reserved;
        state.totals.bytes = bytes;
        self.outstanding = false;
        self.admission.admitted(&state);
        Ok(())
    }
}
impl Drop for ChunkReservation<'_, '_> {
    fn drop(&mut self) {
        if !self.outstanding {
            return;
        }
        let mut state = match self.admission.state.lock() {
            Ok(state) => state,
            Err(poison) => {
                let mut state = poison.into_inner();
                state.cancelled = true;
                state
            }
        };
        if let Some(reserved) = state.reserved.checked_sub(self.amount) {
            state.reserved = reserved;
        } else {
            // Never wrap or invent a successful zero balance on inconsistent state.
            state.cancelled = true;
        }
        self.outstanding = false;
    }
}
pub(super) fn append_checked(
    destination: &mut Vec<u8>,
    actual_chunk: &[u8],
    admission: &Admission,
) -> Result<(), DevMapError> {
    destination
        .len()
        .checked_add(actual_chunk.len())
        .filter(|n| *n <= MAX_FILE_BYTES)
        .ok_or_else(|| fail("origin proof file limit"))?;
    let token = admission.reserve_chunk(actual_chunk.len())?;
    destination
        .try_reserve_exact(actual_chunk.len())
        .map_err(|_| fail("origin proof file allocation failed"))?;
    token.commit_before_append()?;
    // Allocation is complete and the accepted charge is monotone before length changes.
    destination.extend_from_slice(actual_chunk);
    Ok(())
}
pub(super) fn read_admitted<R: Read>(
    reader: &mut R,
    admission: &Admission,
    scratch: &mut [u8; SCRATCH_BYTES],
) -> Result<Vec<u8>, DevMapError> {
    let mut destination = Vec::new();
    loop {
        let allowed = (MAX_FILE_BYTES - destination.len() + 1).min(SCRATCH_BYTES);
        let n = match reader.read(&mut scratch[..allowed]) {
            Ok(n) => n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        if n == 0 {
            return Ok(destination);
        }
        // Defensive check for an invalid Read implementation before slicing scratch.
        if n > allowed {
            return Err(fail("origin proof invalid read length"));
        }
        append_checked(&mut destination, &scratch[..n], admission)?;
    }
}
#[cfg(test)]
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
struct Snapshot {
    totals: AdmissionTotals,
    reserved: usize,
    cancelled: bool,
}

#[cfg(test)]
#[path = "origin_cache_admission_red_tests.rs"]
mod parallel_admission_tests;
