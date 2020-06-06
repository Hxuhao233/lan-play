pub use std::io;
pub use std::net::SocketAddr;
pub use self::direct::DirectProxy;

mod direct;
pub type BoxTcp = Box<dyn socket::Tcp + Unpin>;
pub type BoxUdp = Box<dyn socket::Udp + Unpin>;
pub type BoxProxy = Box<dyn Proxy + Unpin>;

pub mod socket {
    use std::net::SocketAddr;
    use tokio::io::{self, AsyncRead, AsyncWrite};

    #[async_trait(?Send)]
    pub trait Tcp: AsyncRead + AsyncWrite {
    }

    #[async_trait(?Send)]
    pub trait Udp {
        async fn send_to(&mut self, buf: &[u8], addr: SocketAddr) -> io::Result<usize>;
        async fn recv_from(&mut self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)>;
    }
}
#[async_trait(?Send)]
pub trait Proxy
{
    async fn new_tcp(&mut self, addr: SocketAddr) -> io::Result<BoxTcp>;
    async fn new_udp(&mut self, addr: SocketAddr) -> io::Result<BoxUdp>;
    async fn join(&mut self) -> ();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio::prelude::*;

    async fn server_tcp() -> (TcpListener, u16) {
        let mut server = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = server.local_addr().unwrap().port();
        (server, port)
    }

    #[tokio::test]
    async fn test_direct_proxy() -> tokio::io::Result<()> {
        let (mut server, port) = server_tcp().await;
        let join = tokio::spawn(async move {
            let (mut socket, _) = server.accept().await?;
            let mut buf = [0u8; 5];
            socket.read_exact(&mut buf).await?;
            assert_eq!(&buf, b"hello");
            socket.write_all(b"world").await?;
            Ok::<_, tokio::io::Error>(())
        });
        let mut proxy: BoxProxy = DirectProxy::new();
        let mut tcp = proxy.new_tcp(
            SocketAddr::new("127.0.0.1".parse().unwrap(), port)
        ).await.unwrap();
        let mut buf = [0u8; 5];
        tcp.write_all(b"hello").await?;
        tcp.read_exact(&mut buf).await?;
        assert_eq!(&buf, b"world");

        join.await.unwrap().unwrap();
        Ok(())
    }
}
