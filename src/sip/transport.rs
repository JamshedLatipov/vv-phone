use tokio::net::UdpSocket;
use std::net::SocketAddr;
use anyhow::Result;

pub struct SipUdpTransport {
    socket: UdpSocket,
}

impl SipUdpTransport {
    pub async fn new(bind_addr: &str) -> Result<Self> {
        let socket = UdpSocket::bind(bind_addr).await?;
        Ok(Self { socket })
    }

    pub async fn send_to(&self, data: &[u8], addr: SocketAddr) -> Result<usize> {
        Ok(self.socket.send_to(data, addr).await?)
    }

    pub async fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
        Ok(self.socket.recv_from(buf).await?)
    }
}
