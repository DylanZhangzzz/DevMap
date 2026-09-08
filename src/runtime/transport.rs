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

pub const EXCHANGE_DEADLINE: Duration = Duration::from_secs(30);

pub fn transfer(bytes: &[u8], request_id: u64, owner_instance: &str) -> super::protocol::Transfer {
    super::protocol::Transfer {
        request_id,
        owner_instance: owner_instance.to_owned(),
        total: bytes.len(),
        digest: crate::canonical::sha256_hex(bytes),
    }
}
pub fn validate_transfer(tag: &super::protocol::Transfer, limit: usize) -> io::Result<()> {
    if tag.total == 0
        || tag.total > limit
        || tag.digest.len() != 64
        || !tag.digest.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid("runtime aggregate size or digest rejected"));
    }
    Ok(())
}
pub async fn send_bytes(
    stream: &mut (impl AsyncWrite + Unpin),
    tag: &super::protocol::Transfer,
    bytes: &[u8],
) -> io::Result<()> {
    for (index, bytes) in bytes.chunks(super::protocol::CHUNK_BYTES).enumerate() {
        write(
            stream,
            &super::protocol::Chunk {
                transfer: tag.clone(),
                offset: index * super::protocol::CHUNK_BYTES,
                bytes: bytes.to_vec(),
            },
        )
        .await?;
    }
    Ok(())
}
pub async fn receive_bytes(
    stream: &mut (impl AsyncRead + Unpin),
    tag: &super::protocol::Transfer,
    limit: usize,
) -> io::Result<Vec<u8>> {
    validate_transfer(tag, limit)?;
    let mut bytes = Vec::with_capacity(tag.total);
    while bytes.len() < tag.total {
        let chunk: super::protocol::Chunk = read(stream, IO_DEADLINE).await?;
        if chunk.transfer != *tag
            || chunk.offset != bytes.len()
            || chunk.bytes.is_empty()
            || chunk.bytes.len() > super::protocol::CHUNK_BYTES
            || chunk.bytes.len() > tag.total - bytes.len()
        {
            return Err(invalid("runtime transfer identity or offset rejected"));
        }
        bytes.extend_from_slice(&chunk.bytes);
    }
    if crate::canonical::sha256_hex(&bytes) != tag.digest {
        return Err(invalid("runtime transfer digest rejected"));
    }
    Ok(bytes)
}
pub fn bounded_json(value: &impl Serialize, limit: usize) -> io::Result<Vec<u8>> {
    struct Limited {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl io::Write for Limited {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            if data.len() > self.limit - self.bytes.len() {
                return Err(invalid("runtime aggregate result limit"));
            }
            self.bytes.extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut out = Limited {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut out, value).map_err(|e| invalid(e.to_string()))?;
    Ok(out.bytes)
}

pub async fn exchange(
    stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    hello: &super::protocol::Hello,
    welcome: &super::protocol::Welcome,
    request_id: u64,
    bytes: Vec<u8>,
) -> Result<super::protocol::ApplicationResult, super::RuntimeCallError> {
    use super::{
        RuntimeCallError,
        protocol::{ApplicationResult, ExchangeReply, Request},
    };
    tokio::time::timeout(EXCHANGE_DEADLINE, async {
        let tag = transfer(&bytes, request_id, &welcome.owner_instance);
        validate_transfer(&tag, super::protocol::MAX_REQUEST)?;
        write(
            stream,
            &Request::Begin {
                protocol: hello.protocol,
                repository: hello.repository.clone(),
                client_instance: hello.client_instance.clone(),
                request_id,
                owner_instance: tag.owner_instance.clone(),
                total: tag.total,
                digest: tag.digest.clone(),
            },
        )
        .await?;
        match read(stream, IO_DEADLINE).await? {
            ExchangeReply::Ready {
                request_id: actual,
                owner_instance,
            } if actual == request_id && owner_instance == welcome.owner_instance => {}
            ExchangeReply::Busy {
                request_id: actual,
                owner_instance,
            } if actual == request_id && owner_instance == welcome.owner_instance => {
                return Err(RuntimeCallError::Busy);
            }
            _ => return Err(invalid("runtime admission identity mismatch").into()),
        }
        send_bytes(stream, &tag, &bytes).await?;
        drop(bytes);
        let result_tag = match read(stream, EXCHANGE_DEADLINE).await? {
            ExchangeReply::Result { transfer }
                if transfer.request_id == request_id
                    && transfer.owner_instance == welcome.owner_instance =>
            {
                transfer
            }
            ExchangeReply::Busy {
                request_id: actual,
                owner_instance,
            } if actual == request_id && owner_instance == welcome.owner_instance => {
                return Err(RuntimeCallError::Busy);
            }
            _ => return Err(invalid("runtime result identity mismatch").into()),
        };
        let result = receive_bytes(stream, &result_tag, super::protocol::MAX_RESULT).await?;
        let result: ApplicationResult =
            serde_json::from_slice(&result).map_err(|e| invalid(e.to_string()))?;
        match result {
            ApplicationResult::Error { error } => Err(RuntimeCallError::Domain(error)),
            result => Ok(result),
        }
    })
    .await
    .map_err(|_| {
        RuntimeCallError::Transport(io::Error::new(
            io::ErrorKind::TimedOut,
            "runtime exchange deadline",
        ))
    })?
}

#[cfg(test)]
mod exchange_tests {
    use super::super::protocol::*;
    use super::*;
    #[test]
    fn duplex_exchange_preserves_domain_message_and_detects_result_identity() {
        super::super::reactor().unwrap().block_on(async {
            for wrong_owner in [false, true] {
                let (mut client, mut server) = tokio::io::duplex(MAX_FRAME * 2);
                let hello = Hello {
                    protocol: VERSION,
                    repository: "repo".into(),
                    build: "build".into(),
                    source: ".".into(),
                    git_dir: ".".into(),
                    client_instance: "client".into(),
                };
                let welcome = Welcome {
                    protocol: VERSION,
                    repository: "repo".into(),
                    build: "build".into(),
                    owner_instance: "owner".into(),
                    owner_pid: 1,
                    client_instance: "client".into(),
                };
                let task = tokio::spawn(async move {
                    let request: Request = read(&mut server, IO_DEADLINE).await.unwrap();
                    let Request::Begin {
                        request_id,
                        owner_instance,
                        total,
                        digest,
                        ..
                    } = request
                    else {
                        panic!()
                    };
                    write(
                        &mut server,
                        &ExchangeReply::Ready {
                            request_id,
                            owner_instance: owner_instance.clone(),
                        },
                    )
                    .await
                    .unwrap();
                    let tag = Transfer {
                        request_id,
                        owner_instance,
                        total,
                        digest,
                    };
                    let bytes = receive_bytes(&mut server, &tag, MAX_REQUEST).await.unwrap();
                    assert_eq!(bytes.len(), 20000);
                    let result = bounded_json(
                        &ApplicationResult::Error {
                            error: DomainError::Domain {
                                message: "repository store: exact original diagnostic".into(),
                            },
                        },
                        MAX_RESULT,
                    )
                    .unwrap();
                    let tag = transfer(
                        &result,
                        request_id,
                        if wrong_owner { "replacement" } else { "owner" },
                    );
                    write(
                        &mut server,
                        &ExchangeReply::Result {
                            transfer: tag.clone(),
                        },
                    )
                    .await
                    .unwrap();
                    if !wrong_owner {
                        send_bytes(&mut server, &tag, &result).await.unwrap();
                    }
                });
                let error = exchange(&mut client, &hello, &welcome, 1, vec![b'x'; 20000])
                    .await
                    .unwrap_err();
                if wrong_owner {
                    assert!(matches!(
                        error,
                        super::super::RuntimeCallError::Transport(_)
                    ));
                } else {
                    assert!(matches!(error, super::super::RuntimeCallError::Domain(_)));
                    assert_eq!(
                        error.to_string(),
                        "repository store: exact original diagnostic"
                    );
                }
                task.await.unwrap();
            }
        });
    }
}
