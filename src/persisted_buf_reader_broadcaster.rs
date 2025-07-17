use bytes::Bytes;
use log::{debug, error, info};
use std::collections::HashMap;
use std::io::BufRead;
use std::io::Write;
use std::sync::{Arc, RwLock};
use tokio::sync::Mutex;
use uuid::Uuid;

use tokio::sync::mpsc::{Receiver, Sender};

/// This struct reads from a BufRead and broadcasts the lines to all receivers.
/// It is aimed to be used for stdout/stderr streams, we need two things:
///   - Being able to read the stdout/stderr from several places.
///   - Being able to read the stdout/stderr from the beginning, even though
///     the stream has already started and you connect later.
///
/// It also optionally writes the data to stdout or stderr, depending on the output mode
/// apart from broadcasting it to the receivers, defaults to None.
///
/// # Example
///
/// ```
/// use std::io::BufReader;
/// use lib::persisted_buf_reader_broadcaster::PersistedBufReaderBroadcaster;
///
/// # tokio_test::block_on(async {
/// let buffer = BufReader::new("foo\nbar\n".as_bytes());
/// let mut broadcaster = PersistedBufReaderBroadcaster::new().await;
/// broadcaster.watch(buffer).await;
/// let mut receiver = broadcaster.subscribe().await;
/// assert_eq!(receiver.recv().await.unwrap(), "foo\n".to_string());
/// # })
/// ```
///

#[derive(Clone, Default, Debug)]
pub enum OutputMode {
    Stdout,
    Stderr,

    #[default]
    None,
}

#[derive(Clone, Default)]
pub struct PersistedBufReaderBroadcaster {
    channel_set: ChannelSet,
    join_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    cancel_token: tokio_util::sync::CancellationToken,
    output_mode: OutputMode,
}

impl std::fmt::Debug for PersistedBufReaderBroadcaster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersistedBufReaderBroadcaster")
            .field("channel_set", &self.channel_set)
            .field("join_handle", &self.join_handle)
            .field("cancel_token", &self.cancel_token)
            .finish()
    }
}

impl PersistedBufReaderBroadcaster {
    /// Creates a new PersistedBufReaderBroadcaster from a BufRead.
    /// It will own the passed BufRead and read from it until EOF inside a tokio task.
    ///
    /// # Arguments
    ///
    /// * `buffer` - A BufRead that will be read from.
    ///
    /// # Example
    ///
    /// ```
    /// # tokio_test::block_on(async {
    /// use std::io::BufReader;
    /// use lib::persisted_buf_reader_broadcaster::PersistedBufReaderBroadcaster;
    ///
    /// let buffer = BufReader::new("foo\nbar\n".as_bytes());
    /// let mut broadcaster = PersistedBufReaderBroadcaster::new().await;
    /// broadcaster.watch(buffer).await;
    /// let mut receiver = broadcaster.subscribe().await;
    /// assert_eq!(receiver.recv().await.unwrap(), "foo\n".to_string());
    /// assert_eq!(receiver.recv().await.unwrap(), "bar\n".to_string());
    /// # })
    /// ```
    pub async fn new() -> Self {
        let channel_set = ChannelSet::new().await;

        Self {
            channel_set,
            join_handle: Arc::new(Mutex::new(None)),
            cancel_token: tokio_util::sync::CancellationToken::new(),
            output_mode: OutputMode::None,
        }
    }

    pub fn set_output_mode(&mut self, output_mode: OutputMode) {
        self.output_mode = output_mode;
    }

    pub async fn close(&mut self) {
        self.cancel_token.cancel();
        self.channel_set.close().await;
        self.wait().await;
    }

    /// Starts a tokio task that reads from the buffer and broadcasts the lines to all receivers.
    /// If there is already a buffer being read, it discards it and starts reading from the new buffer.
    pub async fn watch<T: BufRead + Sync + Send + 'static>(&mut self, mut buffer: T) {
        if let Some(join_handle) = self.join_handle.lock().await.take() {
            if !self.cancel_token.is_cancelled() {
                debug!("Cancelling the current reader task as there is already one.");
                self.cancel_token.cancel();
                join_handle.await.unwrap();
            }
            self.cancel_token = tokio_util::sync::CancellationToken::new();
        }

        let join_handle = tokio::spawn({
            let cancel_token = self.cancel_token.clone();
            let channel_set = self.channel_set.clone();
            let mut buf = Vec::new();
            let output_mode = self.output_mode.clone();
            async move {
                loop {
                    tokio::select! {
                        _ = cancel_token.cancelled() => {
                            debug!("Cancellation request received, stopping the task.");
                            break;
                        }
                        _ = async {
                            // we use read_until because we want to be able to read binary data (terminal escapes sequences?)
                            debug!("Going to read from the buffer.");
                            if buffer.read_until(b'\n', &mut buf).unwrap() == 0 {
                                debug!("EOF reached, stopping the task.");
                                cancel_token.cancel();
                            } else {
                                if !matches!(output_mode, OutputMode::None) {
                                    Self::write_to_static_output(&output_mode, buf.clone()).await;
                                }

                                channel_set.broadcast(buf.clone().into()).await;
                                buf.clear();
                                debug!("Data sent");
                            }
                        } => {}
                    }
                }
                info!("Task finished.");
            }
        });

        self.join_handle = Arc::new(Mutex::new(Some(join_handle)));
    }

    async fn write_to_static_output(output_mode: &OutputMode, buf: Vec<u8>) {
        match output_mode {
            OutputMode::Stdout => {
                if let Err(e) = std::io::stdout().write_all(&buf) {
                    error!("Error writing to stdout: {}", e);
                }
            }
            OutputMode::Stderr => {
                if let Err(e) = std::io::stderr().write_all(&buf) {
                    error!("Error writing to stderr: {}", e);
                }
            }
            OutputMode::None => {}
        }
    }

    /// Waits for the tokio task to finish.
    async fn wait(&mut self) {
        let mut join_handle = self.join_handle.lock().await;
        if join_handle.is_some() {
            join_handle.take().unwrap().await.unwrap();
        }
    }

    /// Returns a new receiver that will receive all the data that has been read so far + all the new data.
    pub async fn subscribe(&self) -> BufferReceiver {
        let (sender, receiver) = self.new_channel().await;
        let id = self.channel_set.add_sender(sender).await;

        BufferReceiver { receiver, id }
    }

    /// Unsubscribes a receiver.
    pub async fn unsubscribe(&self, receiver: BufferReceiver) {
        self.channel_set.drop_sender(receiver).await;
    }

    async fn new_channel(&self) -> (Sender<Bytes>, Receiver<Bytes>) {
        tokio::sync::mpsc::channel(1)
    }
}

// Wraps a Receiver<Bytes> and an id that will be used
// to store the sender.
#[derive(Debug)]
pub struct BufferReceiver {
    receiver: Receiver<Bytes>,
    id: Uuid,
}

impl BufferReceiver {
    pub async fn recv(&mut self) -> Option<Bytes> {
        self.receiver.recv().await
    }
}

// This struct is mainly for locking/releasing the data.
// as getters have their own scope, the lock is released
// when the getter returns. This is convenient to avoid
// creating scopes.
// @TODO: Store the historic data in a file.
#[derive(Clone, Debug, Default)]
struct HistoricData {
    data: Arc<RwLock<Vec<Bytes>>>,
}

impl HistoricData {
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
}

#[derive(Clone, Debug, Default)]
struct ChannelSet {
    // A lisk of senders that will receive the data.
    senders: Arc<RwLock<HashMap<Uuid, Sender<Bytes>>>>,

    // The lock is when adding a new sender to avoid having the historical data interleaved with the new data.
    // This way we can guarantee that the no new data is sent to the new sender until the historical data is sent.
    lock: Arc<Mutex<()>>,

    sent_data: HistoricData,
}

/// This is a way of broadcasting data to several receivers.
impl ChannelSet {
    pub async fn new() -> Self {
        Self {
            senders: Arc::new(RwLock::new(HashMap::new())),
            lock: Arc::new(Mutex::new(())),
            ..Default::default()
        }
    }

    pub async fn close(&self) {
        debug!("Closing the channel set.");

        let _lock = self.lock.lock().await;
        self.senders.write().unwrap().clear();
    }

    pub async fn broadcast(&self, data: Bytes) {
        let _lock = self.lock.lock().await;

        let senders = { self.senders.read().unwrap().clone() };
        debug!("Broadcasting data to {} receivers.", senders.len());
        self.sent_data.write(data.clone());
        for (id, sender) in senders {
            if let Ok(permit) = sender.reserve().await {
                permit.send(data.clone());
            } else {
                error!("Error sending data to receiver {}", id);
            }
        }
    }

    /// Adds a new sender to the list of receivers.
    /// It allows sending some initial data to the sender, this is useful when connecting to a stream that has already started.
    /// Returns a UUID that can be used to identify the sender (and delete it later).
    pub async fn add_sender(&self, sender: Sender<Bytes>) -> Uuid {
        let _lock = self.lock.lock().await;

        let initial_data = self.sent_data.read();
        if !initial_data.is_empty() {
            info!("Sending initial data to the new sender.");
            match sender.reserve().await {
                Ok(permit) => permit.send(initial_data),
                Err(e) => error!("Error sending data: {:?}", e),
            }
        }

        let uuid = Uuid::new_v4();
        self.senders.write().unwrap().insert(uuid, sender);
        debug!("Sender added to the list of receivers.");
        uuid
    }

    /// Drops a sender from the list of senders given its reciver
    pub async fn drop_sender(&self, receiver: BufferReceiver) -> Option<Sender<Bytes>> {
        self.senders.write().unwrap().remove(&receiver.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::StdoutWriter;
    use std::io::{BufReader, BufWriter, Write};

    #[tokio::test]
    async fn test_persisted_buf_reader_broadcaster() {
        let buffer = BufReader::new("foo\nbar\n".as_bytes());
        let mut broadcaster = PersistedBufReaderBroadcaster::new().await;
        broadcaster.watch(buffer).await;

        let mut receiver = broadcaster.subscribe().await;
        assert_eq!(receiver.recv().await.unwrap(), "foo\n".to_string());
        assert_eq!(receiver.recv().await.unwrap(), "bar\n".to_string());
        broadcaster.close().await;
    }

    #[tokio::test]
    async fn test_persisted_buf_historic_data() {
        let buffer = BufReader::new("foo\nbar\n".as_bytes());
        let mut broadcaster = PersistedBufReaderBroadcaster::new().await;
        broadcaster.watch(buffer).await;

        let mut receiver = broadcaster.subscribe().await;
        assert_eq!(receiver.recv().await.unwrap(), "foo\n".to_string());
        let mut receiver2 = broadcaster.subscribe().await;
        assert_eq!(receiver2.recv().await.unwrap(), "foo\nbar\n".to_string());
        broadcaster.close().await;
    }

    #[tokio::test]
    async fn test_dual_watch_historic_data() {
        let buffer = BufReader::new("foo\nbar\n".as_bytes());
        let mut broadcaster = PersistedBufReaderBroadcaster::new().await;
        let mut receiver = broadcaster.subscribe().await;
        broadcaster.watch(buffer).await;

        let buffer = BufReader::new("foo2\nbar2\n".as_bytes());
        assert_eq!(receiver.recv().await.unwrap(), "foo\n".to_string());
        assert_eq!(receiver.recv().await.unwrap(), "bar\n".to_string());
        broadcaster.watch(buffer).await;

        assert_eq!(receiver.recv().await.unwrap(), "foo2\n".to_string());
        assert_eq!(receiver.recv().await.unwrap(), "bar2\n".to_string());
        broadcaster.close().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn test_persisted_buf_reader_broadcaster_with_binary_data() {
        let data = b"\xFFF\n";
        let buffer = BufReader::new(data.as_ref());
        let mut broadcaster = PersistedBufReaderBroadcaster::new().await;
        broadcaster.watch(buffer).await;
        info!("subscribing the buffer.");

        let mut receiver = broadcaster.subscribe().await;

        assert_eq!(receiver.recv().await, Some(Bytes::from(data.to_vec())));
        info!("Closing the broadcaster from the terst.");

        broadcaster.close().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn test_subsequent_messages() {
        let data = b"\xFFF\n";
        let buffer = BufReader::new(data.as_ref());
        let mut broadcaster = PersistedBufReaderBroadcaster::new().await;
        broadcaster.watch(buffer).await;
        let mut receiver = broadcaster.subscribe().await;

        assert_eq!(receiver.recv().await, Some(Bytes::from(data.to_vec())));
        broadcaster.close().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn test_persisted_buf_reader_broadcaster_with_utf8mb4() {
        let data = b"\xF0\x9F\x98\x82\n";
        let buffer = BufReader::new(data.as_ref());

        let mut broadcaster = PersistedBufReaderBroadcaster::new().await;
        broadcaster.watch(buffer).await;
        let mut receiver = broadcaster.subscribe().await;

        assert_eq!(receiver.recv().await, Some(Bytes::from(data.to_vec())));
        broadcaster.close().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 20)]
    async fn test_write_read() {
        let (stdout_writer, mut child) = StdoutWriter::new();
        let mut writer = BufWriter::new(stdout_writer.stdin);

        let mut broadcaster = PersistedBufReaderBroadcaster::new().await;
        broadcaster.watch(stdout_writer.stdout).await;

        let mut receiver = broadcaster.subscribe().await;

        let data: &[u8] = b"foo\n";
        writer.write_all(b"foo\n").expect("write failed");
        writer.flush().expect("flush failed");
        assert_eq!(receiver.recv().await, Some(Bytes::from(data.to_vec())));

        let data: &[u8] = b"bar\n";
        writer.write_all(data).expect("write failed");
        writer.flush().expect("flush failed");
        assert_eq!(receiver.recv().await, Some(Bytes::from(data.to_vec())));

        let mut new_receiver = broadcaster.subscribe().await;
        assert_eq!(
            new_receiver.recv().await,
            Some(Bytes::from(b"foo\nbar\n".to_vec()))
        );
        let data: &[u8] = b"bar\n";
        writer.write_all(data).expect("write failed");
        writer.flush().expect("flush failed");
        assert_eq!(receiver.recv().await, Some(Bytes::from(data.to_vec())));
        assert_eq!(new_receiver.recv().await, Some(Bytes::from(data.to_vec())));

        child.kill().expect("kill failed");
        broadcaster.close().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 20)]
    async fn test_write_to_stdout() {
        let (stdout_writer, mut child) = StdoutWriter::new();
        let mut writer = BufWriter::new(stdout_writer.stdin);

        let mut broadcaster = PersistedBufReaderBroadcaster::new().await;
        broadcaster.set_output_mode(OutputMode::Stdout);
        broadcaster.watch(stdout_writer.stdout).await;

        let output = crate::test_utils::capture_stdout(|| {
            writer.write_all(b"foo\n").expect("write failed");
            writer.flush().expect("flush failed");
            std::thread::sleep(std::time::Duration::from_millis(100));
        });
        assert!(output.contains("foo\n"), "Output should contain 'foo\\n'");

        child.kill().expect("kill failed");
        broadcaster.close().await;
    }
}
