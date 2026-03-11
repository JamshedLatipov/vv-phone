use tokio::net::{UdpSocket, TcpStream, TcpListener};
use std::net::SocketAddr;
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use std::collections::HashMap;

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
    listener: TcpListener,
    connections: Arc<Mutex<HashMap<SocketAddr, Arc<Mutex<TcpStream>>>>>,
}

impl SipTcpTransport {
    pub async fn new(bind_addr: &str) -> Result<Self> {
        let listener = TcpListener::bind(bind_addr).await?;
        Ok(Self {
            listener,
            connections: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    async fn get_or_create_connection(&self, addr: SocketAddr) -> Result<Arc<Mutex<TcpStream>>> {
        let mut conns = self.connections.lock().await;
        if let Some(conn) = conns.get(&addr) {
            return Ok(conn.clone());
        }

        let stream = TcpStream::connect(addr).await?;
        let conn = Arc::new(Mutex::new(stream));
        conns.insert(addr, conn.clone());
        Ok(conn)
    }
}

#[async_trait]
impl SipTransport for SipTcpTransport {
    async fn send_to(&self, data: &[u8], addr: SocketAddr) -> Result<usize> {
        let conn = self.get_or_create_connection(addr).await?;
        let mut stream = conn.lock().await;
        stream.write_all(data).await?;
        Ok(data.len())
    }

    async fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
        // Simplified: wait for a new connection or use existing ones.
        // In a real commercial client, this would be a more complex event loop.
        let (mut stream, addr) = self.listener.accept().await?;
        let n = stream.read(buf).await?;

        let mut conns = self.connections.lock().await;
        conns.insert(addr, Arc::new(Mutex::new(stream)));

        Ok((n, addr))
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.listener.local_addr()?)
    }
}
