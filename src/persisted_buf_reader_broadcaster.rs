use bytes::Bytes;
use std::io::BufRead;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc::{Receiver, Sender};

#[derive(Clone)]
pub struct PersistedBufReaderBroadcaster {
    channel_set: ChannelSet,
    sent_data: HistoricData,
    join_handle: Arc<tokio::task::JoinHandle<()>>,
}

// This struct is mainly for locking/releasing the data.
// as getters have their own scope, the lock is released
// when the getter returns. This is convenient to avoid
// creating scopes.
#[derive(Clone)]
struct HistoricData {
    data: Arc<RwLock<Vec<Bytes>>>,
}

impl HistoricData {
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn write(&self, bytes: Bytes) {
        let mut data = self.data.write().unwrap();
        data.push(bytes);
    }

    pub fn read(&self) -> Bytes {
        let mut bytes: bytes::BytesMut = bytes::BytesMut::new();
        for line in self.data.read().unwrap().iter() {
            bytes.extend_from_slice(line);
        }
        bytes.into()
    }

    pub fn len(&self) -> usize {
        self.data.read().unwrap().len()
    }
}

const DEFAULT_LINE_CAPACITY: usize = 1024;

#[derive(Clone)]
struct ChannelSet {
    senders: Arc<RwLock<Vec<Sender<Bytes>>>>,
}

impl ChannelSet {
    pub async fn new() -> Self {
        Self {
            senders: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn broadcast(&self, data: Bytes) {
        let senders = { self.senders.read().unwrap().clone() };
        for sender in senders.iter() {
            sender.send(data.clone()).await.unwrap();
        }
    }

    pub async fn add_sender(&self, sender: Sender<Bytes>) {
        self.senders.write().unwrap().push(sender);
    }
}

/// This struct reads from a BufRead and broadcasts the lines to all receivers.
/// It is aimed to be used for stdout/stderr streams, we need two things:
///   - Being able to read the stdout/stderr from several places.
///   - Being able to read the stdout/stderr from the beginning, even though
///     the stream has already started and you connect later.
///
/// # Example
///
/// ```
/// use std::io::BufReader;
/// use async_buf_reader_broadcaster::PersistedBufReaderBroadcaster;
///
/// let buffer = BufReader::new("foo\nbar\n".as_bytes());
/// let mut broadcaster = PersistedBufReaderBroadcaster::new(buffer).await;
/// let mut receiver = broadcaster.subscribe().await;
/// assert_eq!(receiver.recv().await.unwrap(), "foo\n".to_string());
/// ```
///
impl PersistedBufReaderBroadcaster {
    /// Creates a new PersistedBufReaderBroadcaster from a BufRead.
    /// It will own the passed BufRead and read from it until EOF.
    ///
    /// # Arguments
    ///
    /// * `buffer` - A BufRead that will be read from.
    ///
    /// # Example
    ///
    /// ```
    /// use std::io::BufReader;
    /// use async_buf_reader_broadcaster::PersistedBufReaderBroadcaster;
    ///
    /// let buffer = BufReader::new("foo\nbar\n".as_bytes());
    /// let mut broadcaster = PersistedBufReaderBroadcaster::new(buffer).await;
    /// ```
    pub async fn new<T: BufRead + Sync + Send + 'static>(mut buffer: T) -> Self {
        let channel_set = ChannelSet::new().await;
        let sent_data = HistoricData::new();

        let join_handle = tokio::spawn({
            let channel_set = channel_set.clone();
            let sent_data = sent_data.clone();
            let mut buf = Vec::new();
            async move {
                // we use read_until because we want to be able to read binary data (terminal escapes sequences?)
                while buffer.read_until(b'\n', &mut buf).unwrap() > 0 {
                    channel_set.broadcast(buf.clone().into()).await;
                    sent_data.write(buf.clone().into());
                    buf.clear();
                }
            }
        });
        Self {
            channel_set,
            sent_data,
            join_handle: Arc::new(join_handle),
        }
    }

    pub async fn wait(&mut self) {
        Arc::get_mut(&mut self.join_handle).unwrap().await.unwrap();
    }

    pub async fn subscribe(&self) -> Receiver<Bytes> {
        let (sender, receiver) = self.new_channel().await;
        let historic_data = self.sent_data.read();
        sender.send(historic_data).await.unwrap();
        self.channel_set.add_sender(sender).await;

        receiver
    }

    async fn new_channel(&self) -> (Sender<Bytes>, Receiver<Bytes>) {
        tokio::sync::mpsc::channel(self.sent_data.len() + DEFAULT_LINE_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    #[tokio::test]
    async fn test_persisted_buf_reader_broadcaster() {
        let buffer = BufReader::new("foo\nbar\n".as_bytes());
        let mut broadcaster = PersistedBufReaderBroadcaster::new(buffer).await;
        broadcaster.wait().await;

        let mut receiver = broadcaster.subscribe().await;
        assert_eq!(receiver.recv().await.unwrap(), "foo\nbar\n".to_string());
    }

    #[tokio::test]
    async fn test_persisted_buf_historic_data() {
        let buffer = BufReader::new("foo\nbar\n".as_bytes());
        let mut broadcaster = PersistedBufReaderBroadcaster::new(buffer).await;
        broadcaster.wait().await;

        let mut receiver = broadcaster.subscribe().await;
        assert_eq!(receiver.recv().await.unwrap(), "foo\nbar\n".to_string());
        let mut receiver2 = broadcaster.subscribe().await;
        assert_eq!(receiver2.recv().await.unwrap(), "foo\nbar\n".to_string());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn test_persisted_buf_reader_broadcaster_with_binary_data() {
        let data = b"\xFFF\n";
        let buffer = BufReader::new(data.as_ref());

        let mut broadcaster = PersistedBufReaderBroadcaster::new(buffer).await;
        broadcaster.wait().await;
        let mut receiver = broadcaster.subscribe().await;

        assert_eq!(receiver.recv().await, Some(Bytes::from(data.to_vec())));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn test_persisted_buf_reader_broadcaster_with_utf8mb4() {
        let data = b"\xF0\x9F\x98\x82\n";
        let buffer = BufReader::new(data.as_ref());

        let mut broadcaster = PersistedBufReaderBroadcaster::new(buffer).await;
        broadcaster.wait().await;
        let mut receiver = broadcaster.subscribe().await;

        assert_eq!(receiver.recv().await, Some(Bytes::from(data.to_vec())));
    }
}
