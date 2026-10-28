use anyhow::{Result, bail};
use one_mk_proto::{Control, StreamKind};
use tokio::sync::mpsc;

/// Reads one length-prefixed control frame. `Ok(None)` on clean end of stream.
pub async fn read_frame(recv: &mut quinn::RecvStream, buf: &mut Vec<u8>) -> Result<Option<Control>> {
    let mut len = [0u8; 2];
    match recv.read_exact(&mut len).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u16::from_le_bytes(len) as usize;
    if len > Control::MAX_LEN {
        bail!("control frame too large ({len} bytes)");
    }
    buf.resize(len, 0);
    recv.read_exact(buf).await?;
    Ok(Some(Control::decode(buf)?))
}

/// Sends control messages from any task without blocking it; a background
/// task batches whatever is queued into a single write.
#[derive(Clone)]
pub struct ControlWriter {
    tx: mpsc::UnboundedSender<Control>,
}

impl ControlWriter {
    /// Takes ownership of the control stream (the `StreamKind` byte must
    /// already have been written by the opener).
    pub fn spawn(mut send: quinn::SendStream) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<Control>();
        tokio::spawn(async move {
            let mut buf = Vec::with_capacity(256);
            while let Some(msg) = rx.recv().await {
                buf.clear();
                msg.encode_frame(&mut buf);
                while let Ok(more) = rx.try_recv() {
                    more.encode_frame(&mut buf);
                }
                if send.write_all(&buf).await.is_err() {
                    break;
                }
            }
            let _ = send.finish();
        });
        Self { tx }
    }

    /// Queues a message. Returns false once the connection is gone.
    pub fn send(&self, msg: Control) -> bool {
        self.tx.send(msg).is_ok()
    }
}

/// Opens a stream and announces what it carries.
pub(crate) async fn open(conn: &quinn::Connection, kind: StreamKind) -> Result<(quinn::SendStream, quinn::RecvStream)> {
    let (mut send, recv) = conn.open_bi().await?;
    send.write_all(&[kind as u8]).await?;
    Ok((send, recv))
}

/// Accepts the next stream and reads what it carries.
pub(crate) async fn accept(conn: &quinn::Connection) -> Result<(StreamKind, quinn::SendStream, quinn::RecvStream)> {
    let (send, mut recv) = conn.accept_bi().await?;
    let mut kind = [0u8; 1];
    recv.read_exact(&mut kind).await?;
    let Some(kind) = StreamKind::from_u8(kind[0]) else {
        bail!("unknown stream kind {}", kind[0]);
    };
    Ok((kind, send, recv))
}

impl crate::Peer {
    pub async fn open_stream(&self, kind: StreamKind) -> Result<(quinn::SendStream, quinn::RecvStream)> {
        open(&self.conn, kind).await
    }

    pub async fn accept_stream(&self) -> Result<(StreamKind, quinn::SendStream, quinn::RecvStream)> {
        accept(&self.conn).await
    }
}
