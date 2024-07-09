use chrono::{DateTime, Duration};
use chrono_tz::{Tz, GB};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embassy_time::Instant;
use static_cell::make_static;

/// Hold a reference to the time state that can be updated via an NTP task.
pub struct Time {
    /// The time last pulled from NTP.
    sys_start: Mutex<CriticalSectionRawMutex, DateTime<Tz>>,
}

impl Time {
    /// Create the static ref to time state.
    /// Must only be called once or will panic.
    pub fn new() -> &'static Self {
        make_static!(Self {
            sys_start: Mutex::new(DateTime::UNIX_EPOCH.with_timezone(&GB)),
        })
    }

    /// Set the current time.
    pub async fn set_time(&self, now: DateTime<Tz>) {
        let mut sys_start = self.sys_start.lock().await;
        let elapsed = Instant::now().as_millis();
        *sys_start = now
            .checked_sub_signed(Duration::milliseconds(elapsed as i64))
            .expect("sys_start greater as current_ts");
    }

    /// Get the current time.
    pub async fn now(&self) -> DateTime<Tz> {
        let sys_start = self.sys_start.lock().await;
        let elapsed = Instant::now().as_millis();
        *sys_start + Duration::milliseconds(elapsed as i64)
    }
}

pub mod ntp {
    use chrono::DateTime;
    use chrono_tz::{Tz, GB};
    use embassy_futures::select::select;
    use embassy_net::{
        dns::DnsQueryType,
        udp::{PacketMetadata, UdpSocket},
        IpEndpoint, Stack,
    };
    use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, signal::Signal};
    use embassy_time::Timer;
    use no_std_net::{SocketAddr, ToSocketAddrs};
    use sntpc::{
        async_impl::{get_time, NtpUdpSocket},
        NtpContext, NtpTimestampGenerator,
    };
    use thiserror_no_std::Error;

    use super::Time;

    const POOL_NTP_ADDR: &str = "pool.ntp.org";

    /// Signal for request to sync system with NTP.
    pub static SYNC_SIGNAL: Signal<ThreadModeRawMutex, bool> = Signal::new();

    /// Error enum for NTP request.
    #[derive(Error, Debug)]
    pub enum SntpcError {
        #[error("to_socket_addrs")]
        ToSocketAddrs,
        #[error("no addr")]
        NoAddr,
        #[error("udp send")]
        UdpSend,
        #[error("dns query error")]
        DnsQuery(#[from] embassy_net::dns::Error),
        #[error("dns query error")]
        DnsEmptyResponse,
        #[error("sntc")]
        Sntc(#[from] sntpc::Error),
        #[error("can not parse ntp response")]
        BadNtpResponse,
    }

    impl From<SntpcError> for sntpc::Error {
        fn from(err: SntpcError) -> Self {
            match err {
                SntpcError::ToSocketAddrs => Self::AddressResolve,
                SntpcError::NoAddr => Self::AddressResolve,
                SntpcError::UdpSend => Self::Network,
                _ => todo!(),
            }
        }
    }

    /// UdpSocket wrapper for NTP.
    struct NtpSocket<'a> {
        sock: UdpSocket<'a>,
    }

    impl<'a> NtpUdpSocket for NtpSocket<'a> {
        /// Send buffer via socket.
        async fn send_to<T: ToSocketAddrs + Send>(
            &self,
            buf: &[u8],
            addr: T,
        ) -> sntpc::Result<usize> {
            let mut addr_iter = addr
                .to_socket_addrs()
                .map_err(|_| SntpcError::ToSocketAddrs)?;
            let addr = addr_iter.next().ok_or(SntpcError::NoAddr)?;
            self.sock
                .send_to(buf, sock_addr_to_emb_endpoint(addr))
                .await
                .map_err(|_| SntpcError::UdpSend)
                .unwrap();
            Ok(buf.len())
        }

        /// Receive data from socket.
        async fn recv_from(&self, buf: &mut [u8]) -> sntpc::Result<(usize, SocketAddr)> {
            match self.sock.recv_from(buf).await {
                Ok((size, ip_endpoint)) => Ok((size, emb_endpoint_to_sock_addr(ip_endpoint))),
                Err(_) => panic!("not exp"),
            }
        }
    }

    impl<'a> core::fmt::Debug for NtpSocket<'a> {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("Socket")
                // .field("x", &self.x)
                .finish()
        }
    }

    /// Convert embassy `IpEndpoint` into `SocketAddr`.
    fn emb_endpoint_to_sock_addr(endpoint: IpEndpoint) -> SocketAddr {
        let port = endpoint.port;
        let addr = match endpoint.addr {
            embassy_net::IpAddress::Ipv4(ipv4) => {
                let octets = ipv4.as_bytes();
                let ipv4_addr =
                    no_std_net::Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]);
                no_std_net::IpAddr::V4(ipv4_addr)
            }
            embassy_net::IpAddress::Ipv6(_) => todo!(),
        };
        SocketAddr::new(addr, port)
    }

    /// Convert `SocketAddr` into embassy `IpEndpoint`.
    fn sock_addr_to_emb_endpoint(sock_addr: SocketAddr) -> IpEndpoint {
        let port = sock_addr.port();
        let addr = match sock_addr {
            SocketAddr::V4(addr) => {
                let octets = addr.ip().octets();
                embassy_net::IpAddress::v4(octets[0], octets[1], octets[2], octets[3])
            }
            _ => todo!(),
        };
        IpEndpoint::new(addr, port)
    }

    /// Timestamp generator.
    #[derive(Copy, Clone)]
    struct TimestampGen {
        now: DateTime<Tz>,
    }

    impl TimestampGen {
        /// Take time and convert into timestamp generator.
        async fn new(clock: &Time) -> Self {
            let now = clock.now().await;
            Self { now }
        }
    }

    impl NtpTimestampGenerator for TimestampGen {
        /// Init self.
        fn init(&mut self) {}

        /// Get the timestamp as seconds.
        fn timestamp_sec(&self) -> u64 {
            self.now.timestamp() as u64
        }

        /// Get the timestamp subsec micros.
        fn timestamp_subsec_micros(&self) -> u32 {
            self.now.timestamp_subsec_micros()
        }
    }

    /// NTP task for syncing to NTP.
    #[embassy_executor::task]
    pub async fn ntp_worker(stack: &'static Stack<cyw43::NetDriver<'static>>, time: &'static Time) {
        loop {
            let sleep_sec = match ntp_request(stack, time).await {
                Err(_) => 10,
                Ok(_) => 3600,
            };

            select(Timer::after_secs(sleep_sec), SYNC_SIGNAL.wait()).await;
            SYNC_SIGNAL.reset();
        }
    }

    /// Create an NTP request and set the value in `Time`.
    async fn ntp_request(
        stack: &'static Stack<cyw43::NetDriver<'static>>,
        time: &'static Time,
    ) -> Result<(), SntpcError> {
        let mut addrs = stack.dns_query(POOL_NTP_ADDR, DnsQueryType::A).await?;
        let addr = addrs.pop().ok_or(SntpcError::DnsEmptyResponse)?;

        let octets = addr.as_bytes();
        let ipv4_addr = no_std_net::Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]);
        let sock_addr = SocketAddr::new(no_std_net::IpAddr::V4(ipv4_addr), 123);

        let mut rx_buffer = [0; 4096];
        let mut tx_buffer = [0; 4096];
        let mut rx_meta = [PacketMetadata::EMPTY; 16];
        let mut tx_meta = [PacketMetadata::EMPTY; 16];

        let mut socket = UdpSocket::new(
            stack,
            &mut rx_meta,
            &mut rx_buffer,
            &mut tx_meta,
            &mut tx_buffer,
        );
        socket.bind(1234).unwrap();

        let ntp_socket = NtpSocket { sock: socket };
        let ntp_context = NtpContext::new(TimestampGen::new(time).await);

        let ntp_result = get_time(sock_addr, ntp_socket, ntp_context).await?;
        let now = DateTime::from_timestamp(ntp_result.seconds as i64, 0)
            .ok_or(SntpcError::BadNtpResponse)?;
        let now = now.with_timezone(&GB);
        time.set_time(now).await;

        Ok(())
    }
}
