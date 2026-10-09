//! Length-prefixed JSON framing.
//!
//! Native messaging (browser <-> host) uses a 32-bit length in native byte
//! order followed by UTF-8 JSON. The local IPC channel (host <-> app) uses
//! the same layout with an explicit little-endian length.

use std::io;

use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Largest message accepted from the browser (Chrome allows up to 4 GiB;
/// nothing legitimate in this protocol comes close to this).
pub const MAX_INBOUND: usize = 8 * 1024 * 1024;
/// Browsers reject host -> extension messages above 1 MiB.
pub const MAX_TO_BROWSER: usize = 1024 * 1024;
/// Limit for app <-> host frames.
pub const MAX_IPC: usize = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("message of {0} bytes exceeds the limit")]
    TooLarge(usize),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// Read one frame; `Ok(None)` on clean EOF before a frame starts.
pub async fn read_frame<R: AsyncRead + Unpin>(
    r: &mut R,
    max: usize,
    little_endian: bool,
) -> Result<Option<Vec<u8>>, FrameError> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = if little_endian {
        u32::from_le_bytes(len_buf)
    } else {
        u32::from_ne_bytes(len_buf)
    } as usize;
    if len > max {
        return Err(FrameError::TooLarge(len));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

pub async fn write_frame<W: AsyncWrite + Unpin>(
    w: &mut W,
    payload: &[u8],
    max: usize,
    little_endian: bool,
) -> Result<(), FrameError> {
    if payload.len() > max {
        return Err(FrameError::TooLarge(payload.len()));
    }
    let len = payload.len() as u32;
    let header = if little_endian {
        len.to_le_bytes()
    } else {
        len.to_ne_bytes()
    };
    w.write_all(&header).await?;
    w.write_all(payload).await?;
    w.flush().await?;
    Ok(())
}

/// Native messaging: read a message from the browser.
pub async fn read_native<R: AsyncRead + Unpin>(r: &mut R) -> Result<Option<Vec<u8>>, FrameError> {
    read_frame(r, MAX_INBOUND, false).await
}

/// Native messaging: send a message to the browser.
pub async fn write_native<W: AsyncWrite + Unpin, T: Serialize>(
    w: &mut W,
    msg: &T,
) -> Result<(), FrameError> {
    let payload = serde_json::to_vec(msg)?;
    write_frame(w, &payload, MAX_TO_BROWSER, false).await
}

pub async fn read_ipc<R: AsyncRead + Unpin, T: DeserializeOwned>(
    r: &mut R,
) -> Result<Option<T>, FrameError> {
    match read_frame(r, MAX_IPC, true).await? {
        Some(buf) => Ok(Some(serde_json::from_slice(&buf)?)),
        None => Ok(None),
    }
}

pub async fn write_ipc<W: AsyncWrite + Unpin, T: Serialize>(
    w: &mut W,
    msg: &T,
) -> Result<(), FrameError> {
    let payload = serde_json::to_vec(msg)?;
    write_frame(w, &payload, MAX_IPC, true).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn roundtrip_and_limits() {
        let (mut a, mut b) = tokio::io::duplex(1 << 20);
        write_native(&mut a, &serde_json::json!({"x": 1}))
            .await
            .unwrap();
        let got = read_native(&mut b).await.unwrap().unwrap();
        assert_eq!(got, br#"{"x":1}"#);

        // Oversized length is rejected before allocating.
        let (mut a, mut b) = tokio::io::duplex(64);
        tokio::spawn(async move {
            let _ = a.write_all(&(u32::MAX).to_ne_bytes()).await;
        });
        assert!(matches!(
            read_native(&mut b).await,
            Err(FrameError::TooLarge(_))
        ));

        // Clean EOF.
        let (a, mut b) = tokio::io::duplex(64);
        drop(a);
        assert!(read_native(&mut b).await.unwrap().is_none());
    }
}
