use async_rwlock::RwLock;
use std::io::{BufRead, BufReader, Read};
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};

pub struct PersistedBufReaderBroadcaster {
    senders: Arc<RwLock<Vec<Sender<String>>>>,
    sent_data: Arc<RwLock<Vec<String>>>,
}

const DEFAULT_LINE_CAPACITY: usize = 1024;

impl PersistedBufReaderBroadcaster {
    async fn new_channel(&self) -> (Sender<String>, Receiver<String>) {
        tokio::sync::mpsc::channel(self.sent_data.read().await.len() + DEFAULT_LINE_CAPACITY)
    }

    pub async fn new<T: BufRead + Sync + Send + 'static>(mut buffer: T) -> Self {
        let senders: Arc<RwLock<Vec<Sender<String>>>> = Arc::new(RwLock::new(Vec::new()));
        let sent_data = Arc::new(RwLock::new(Vec::new()));

        tokio::spawn({
            let senders = Arc::clone(&senders);
            let sent_data = Arc::clone(&sent_data);
            let mut line = String::new();
            async move {
                while buffer.read_line(&mut line).unwrap() > 0 {
                    {
                        for sender in senders.read().await.iter() {
                            sender.send(line.clone()).await.unwrap()
                        }
                    }
                    {
                        sent_data.write().await.push(line.clone());
                    }
                    line.clear();
                }
            }
        });

        Self { senders, sent_data }
    }

    pub async fn receiver(&self) -> Receiver<String> {
        println!("Creating receiver");
        let (sender, receiver) = self.new_channel().await;
        {
            self.senders.write().await.push(sender.clone());
        }
        println!("sending old data");
        {
            for data in self.sent_data.read().await.iter() {
                sender.send(data.clone()).await.unwrap();
            }
        }

        receiver
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    async fn test_persisted_buf_reader_broadcaster() {
        let buffer = BufReader::new("foo\nbar\n".as_bytes());

        let mut broadcaster = PersistedBufReaderBroadcaster::new(buffer).await;
        let mut receiver = broadcaster.receiver().await;
        assert_eq!(receiver.recv().await.unwrap(), "foo\n".to_string());

        let mut receiver2 = broadcaster.receiver().await;
        assert_eq!(receiver.recv().await.unwrap(), "bar\n".to_string());
        assert_eq!(receiver2.recv().await.unwrap(), "foo\n".to_string());
        assert_eq!(receiver2.recv().await.unwrap(), "bar\n".to_string());
    }
}
