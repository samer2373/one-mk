use anyhow::{Result, bail};
use one_mk_proto::{ClipHeader, Control, FileMsg, StreamKind};
use tokio::sync::mpsc;

/// Reads one length-prefixed frame body into `buf`. `Ok(false)` on clean
/// end of stream.
async fn read_raw(recv: &mut quinn::RecvStream, buf: &mut Vec<u8>, max: usize) -> Result<bool> {
    let mut len = [0u8; 2];
    match recv.read_exact(&mut len).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(0)) => return Ok(false),
        Err(e) => return Err(e.into()),
    }
    let len = u16::from_le_bytes(len) as usize;
    if len > max {
        bail!("frame too large ({len} bytes)");
    }
    buf.resize(len, 0);
    recv.read_exact(buf).await?;
    Ok(true)
}

/// Reads one length-prefixed control frame. `Ok(None)` on clean end of stream.
pub async fn read_frame(recv: &mut quinn::RecvStream, buf: &mut Vec<u8>) -> Result<Option<Control>> {
    if !read_raw(recv, buf, Control::MAX_LEN).await? {
        return Ok(None);
    }
    Ok(Some(Control::decode(buf)?))
}

/// Reads one file-stream message. `Ok(None)` on clean end of stream.
pub async fn read_file_msg(recv: &mut quinn::RecvStream) -> Result<Option<FileMsg>> {
    let mut buf = Vec::new();
    if !read_raw(recv, &mut buf, FileMsg::MAX_LEN).await? {
        return Ok(None);
    }
    Ok(Some(FileMsg::decode(&buf)?))
}

pub async fn write_file_msg(send: &mut quinn::SendStream, msg: &FileMsg) -> Result<()> {
    let mut buf = Vec::new();
    msg.encode_frame(&mut buf);
    send.write_all(&buf).await?;
    Ok(())
}

/// Reads the header that starts a clipboard stream.
pub async fn read_clip_header(recv: &mut quinn::RecvStream) -> Result<ClipHeader> {
    let mut buf = Vec::new();
    if !read_raw(recv, &mut buf, 1024).await? {
        bail!("clipboard stream ended early");
    }
    Ok(ClipHeader::decode(&buf)?)
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
pub async fn open_stream(conn: &quinn::Connection, kind: StreamKind) -> Result<(quinn::SendStream, quinn::RecvStream)> {
    let (mut send, recv) = conn.open_bi().await?;
    send.write_all(&[kind as u8]).await?;
    Ok((send, recv))
}

/// Reads the byte that says what a freshly accepted stream carries.
pub async fn read_stream_kind(recv: &mut quinn::RecvStream) -> Result<StreamKind> {
    let mut kind = [0u8; 1];
    recv.read_exact(&mut kind).await?;
    let Some(kind) = StreamKind::from_u8(kind[0]) else {
        bail!("unknown stream kind {}", kind[0]);
    };
    Ok(kind)
}

/// Accepts the next stream and reads what it carries.
pub(crate) async fn accept(conn: &quinn::Connection) -> Result<(StreamKind, quinn::SendStream, quinn::RecvStream)> {
    let (send, mut recv) = conn.accept_bi().await?;
    let kind = read_stream_kind(&mut recv).await?;
    Ok((kind, send, recv))
}

impl crate::Peer {
    pub async fn open_stream(&self, kind: StreamKind) -> Result<(quinn::SendStream, quinn::RecvStream)> {
        open_stream(&self.conn, kind).await
    }

    pub async fn accept_stream(&self) -> Result<(StreamKind, quinn::SendStream, quinn::RecvStream)> {
        accept(&self.conn).await
    }
}
