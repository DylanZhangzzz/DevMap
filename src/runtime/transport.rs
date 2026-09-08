use super::protocol::MAX_FRAME;
use serde::{Serialize, de::DeserializeOwned};
use std::{io, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
pub const IO_DEADLINE: Duration = Duration::from_secs(5);
pub fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
pub async fn read<T: DeserializeOwned>(
    stream: &mut (impl AsyncRead + Unpin),
    budget: Duration,
) -> io::Result<T> {
    tokio::time::timeout(budget, async {
        let len = stream.read_u32().await? as usize;
        if len == 0 || len > MAX_FRAME {
            return Err(invalid("runtime frame size rejected"));
        }
        let mut bytes = vec![0; len];
        stream.read_exact(&mut bytes).await?;
        serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "runtime frame deadline"))?
}
pub async fn write<T: Serialize>(
    stream: &mut (impl AsyncWrite + Unpin),
    value: &T,
) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|e| invalid(e.to_string()))?;
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        return Err(invalid("runtime frame size rejected"));
    }
    tokio::time::timeout(IO_DEADLINE, async {
        stream.write_u32(bytes.len() as u32).await?;
        stream.write_all(&bytes).await?;
        stream.flush().await
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "runtime write deadline"))?
}
