use tokio::net::{UdpSocket, TcpStream, TcpListener};
use std::net::SocketAddr;
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, mpsc};
use std::collections::HashMap;
use tokio::net::tcp::{OwnedWriteHalf};

#[async_trait]
pub trait SipTransport: Send + Sync {
    async fn send_to(&self, data: &[u8], addr: SocketAddr) -> Result<usize>;
    async fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr)>;
    fn local_addr(&self) -> Result<SocketAddr>;
}

pub struct SipUdpTransport {
    socket: UdpSocket,
}

impl SipUdpTransport {
    pub async fn new(bind_addr: &str) -> Result<Self> {
        let socket = UdpSocket::bind(bind_addr).await?;
        Ok(Self { socket })
    }
}

#[async_trait]
impl SipTransport for SipUdpTransport {
    async fn send_to(&self, data: &[u8], addr: SocketAddr) -> Result<usize> {
        Ok(self.socket.send_to(data, addr).await?)
    }

    async fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
        Ok(self.socket.recv_from(buf).await?)
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }
}

pub struct SipTcpTransport {
    local_addr: SocketAddr,
    writers: Arc<Mutex<HashMap<SocketAddr, Arc<Mutex<OwnedWriteHalf>>>>>,
    recv_rx: Arc<Mutex<mpsc::Receiver<(Vec<u8>, SocketAddr)>>>,
    recv_tx: mpsc::Sender<(Vec<u8>, SocketAddr)>,
}

impl SipTcpTransport {
    pub async fn new(bind_addr: &str) -> Result<Self> {
        let listener = TcpListener::bind(bind_addr).await?;
        let local_addr = listener.local_addr()?;
        let (recv_tx, recv_rx) = mpsc::channel(100);
        let writers = Arc::new(Mutex::new(HashMap::new()));

        let writers_clone = writers.clone();
        let recv_tx_clone = recv_tx.clone();

        tokio::spawn(async move {
            while let Ok((stream, addr)) = listener.accept().await {
                let (reader, writer) = stream.into_split();
                writers_clone.lock().await.insert(addr, Arc::new(Mutex::new(writer)));
                let tx = recv_tx_clone.clone();
                tokio::spawn(async move {
                    let mut reader = reader;
                    let mut buf = [0u8; 8192];
                    while let Ok(n) = reader.read(&mut buf).await {
                        if n == 0 { break; }
                        if tx.send((buf[..n].to_vec(), addr)).await.is_err() { break; }
                    }
                });
            }
        });

        Ok(Self {
            local_addr,
            writers,
            recv_rx: Arc::new(Mutex::new(recv_rx)),
            recv_tx,
        })
    }

    async fn get_or_create_writer(&self, addr: SocketAddr) -> Result<Arc<Mutex<OwnedWriteHalf>>> {
        let mut writers = self.writers.lock().await;
        if let Some(writer) = writers.get(&addr) {
            return Ok(writer.clone());
        }

        let stream = TcpStream::connect(addr).await?;
        let (reader, writer) = stream.into_split();
        let writer_arc = Arc::new(Mutex::new(writer));
        writers.insert(addr, writer_arc.clone());

        let tx = self.recv_tx.clone();
        tokio::spawn(async move {
            let mut reader = reader;
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf).await {
                if n == 0 { break; }
                if tx.send((buf[..n].to_vec(), addr)).await.is_err() { break; }
            }
        });

        Ok(writer_arc)
    }
}

#[async_trait]
impl SipTransport for SipTcpTransport {
    async fn send_to(&self, data: &[u8], addr: SocketAddr) -> Result<usize> {
        let writer_arc = self.get_or_create_writer(addr).await?;
        let mut writer = writer_arc.lock().await;
        writer.write_all(data).await?;
        Ok(data.len())
    }

    async fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
        let mut rx = self.recv_rx.lock().await;
        if let Some((data, addr)) = rx.recv().await {
            let len = data.len().min(buf.len());
            buf[..len].copy_from_slice(&data[..len]);
            Ok((len, addr))
        } else {
            Err(anyhow::anyhow!("Receive channel closed"))
        }
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.local_addr)
    }
}
