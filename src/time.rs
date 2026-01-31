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
            sys_start: Mutex::new(
                DateTime::from_timestamp(0, 0)
                    .expect("valid timestamp")
                    .with_timezone(&GB)
            ),
        })
    }

    /// Set the current time.
    pub async fn set_time(&self, now: DateTime<Tz>) {
        let mut sys_start = self.sys_start.lock().await;
        let elapsed = Instant::now().as_millis();
        *sys_start = now
            .checked_sub_signed(Duration::milliseconds(elapsed as i64))
            .expect("sys_start calculation overflow");
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
        udp::{PacketMetadata, UdpMetadata, UdpSocket},
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
    const GOOGLE_NTP_IP: [u8; 4] = [216, 239, 35, 0];

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
            log::info!("NTP socket: sending {} bytes", buf.len());
            let mut addr_iter = addr
                .to_socket_addrs()
                .map_err(|_| SntpcError::ToSocketAddrs)?;
            let addr = addr_iter.next().ok_or(SntpcError::NoAddr)?;
            match self
                .sock
                .send_to(buf, sock_addr_to_emb_endpoint(addr))
                .await
            {
                Ok(_) => {
                    log::info!("NTP socket: sent successfully");
                    Ok(buf.len())
                }
                Err(e) => {
                    log::error!("NTP socket: send failed: {:?}", e);
                    Err(SntpcError::UdpSend.into())
                }
            }
        }

        /// Receive data from socket.
        async fn recv_from(&self, buf: &mut [u8]) -> sntpc::Result<(usize, SocketAddr)> {
            log::info!("NTP socket: waiting for response...");
            match self.sock.recv_from(buf).await {
                Ok((size, ip_endpoint)) => {
                    log::info!("NTP socket: received {} bytes", size);
                    Ok((size, emb_endpoint_to_sock_addr(ip_endpoint)))
                }
                Err(e) => {
                    log::error!("NTP socket: receive failed: {:?}", e);
                    Err(sntpc::Error::Network)
                }
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
    fn emb_endpoint_to_sock_addr(endpoint: UdpMetadata) -> SocketAddr {
        let port = endpoint.endpoint.port;
        let addr = match endpoint.endpoint.addr {
            embassy_net::IpAddress::Ipv4(ipv4) => {
                let octets = ipv4.as_octets();
                let ipv4_addr =
                    no_std_net::Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]);
                no_std_net::IpAddr::V4(ipv4_addr)
            }
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

    /// NTP task for syncing to NTP. Waits for network stack to be ready first.
    #[embassy_executor::task]
    pub async fn ntp_worker(time: &'static Time, _app_state: &'static crate::system::SystemState) {
        log::info!("NTP worker: Waiting for network...");
        let stack = crate::network::get_network_stack().await;
        log::info!("NTP worker: Starting");
        loop {
            log::info!("NTP sync triggered");

            let sleep_sec = match ntp_request(stack, time).await {
                Err(e) => {
                    log::error!("NTP request failed: {:?}", e);
                    10
                }
                Ok(_) => {
                    log::info!("NTP sync successful");
                    3600
                }
            };

            select(Timer::after_secs(sleep_sec), SYNC_SIGNAL.wait()).await;
            SYNC_SIGNAL.reset();
        }
    }

    /// Create an NTP request and set the value in `Time`.
    async fn ntp_request(stack: Stack<'static>, time: &'static Time) -> Result<(), SntpcError> {
        log::info!("Starting NTP request");

        // Wait for network stack to be ready
        log::info!("Waiting for network stack to be ready...");
        stack.wait_config_up().await;
        log::info!("Network stack is ready");

        // Try multiple NTP servers in order: DNS, local gateway, external
        let servers_to_try = get_ntp_servers_to_try(stack).await;

        for (i, sock_addr) in servers_to_try.iter().enumerate() {
            log::info!(
                "Trying NTP server {} of {}: {:?}",
                i + 1,
                servers_to_try.len(),
                sock_addr
            );

            match try_ntp_sync(stack, time, *sock_addr).await {
                Ok(()) => {
                    log::info!("NTP sync successful with server: {:?}", sock_addr);
                    return Ok(());
                }
                Err(e) => {
                    log::warn!("NTP sync failed with {:?}: {:?}", sock_addr, e);
                    if i < servers_to_try.len() - 1 {
                        log::info!("Trying next server...");
                    }
                }
            }
        }

        log::error!("All NTP servers failed");
        Err(SntpcError::Sntc(sntpc::Error::Network))
    }

    /// Get list of NTP servers to try in order
    async fn get_ntp_servers_to_try(stack: Stack<'static>) -> heapless::Vec<SocketAddr, 3> {
        let mut servers = heapless::Vec::new();

        // Try DNS first
        if let Ok(addr) = try_dns_query(stack).await {
            log::info!("DNS resolved successfully");
            let _ = servers.push(addr);
        }

        let fallback_addr = SocketAddr::new(
            no_std_net::IpAddr::V4(no_std_net::Ipv4Addr::new(
                GOOGLE_NTP_IP[0],
                GOOGLE_NTP_IP[1],
                GOOGLE_NTP_IP[2],
                GOOGLE_NTP_IP[3],
            )),
            123,
        );
        let _ = servers.push(fallback_addr);

        servers
    }

    /// Try to sync time with a specific NTP server
    async fn try_ntp_sync(
        stack: Stack<'static>,
        time: &'static Time,
        sock_addr: SocketAddr,
    ) -> Result<(), SntpcError> {
        log::info!("Connecting to NTP server: {:?}", sock_addr);

        // NTP packets are only 48 bytes, so 512 bytes is more than enough
        let mut rx_buffer = [0; 512];
        let mut tx_buffer = [0; 512];
        let mut rx_meta = [PacketMetadata::EMPTY; 4];
        let mut tx_meta = [PacketMetadata::EMPTY; 4];

        let mut socket = UdpSocket::new(
            stack,
            &mut rx_meta,
            &mut rx_buffer,
            &mut tx_meta,
            &mut tx_buffer,
        );
        if let Err(e) = socket.bind(1234) {
            log::error!("Failed to bind UDP socket: {:?}", e);
            return Err(SntpcError::Sntc(sntpc::Error::Network));
        }

        log::info!("Socket bound, sending NTP request");

        let ntp_socket = NtpSocket { sock: socket };
        let ntp_context = NtpContext::new(TimestampGen::new(time).await);

        // Add timeout to NTP request (10 seconds)
        let ntp_result = embassy_futures::select::select(
            get_time(sock_addr, ntp_socket, ntp_context),
            Timer::after_secs(10),
        )
        .await;

        let ntp_result = match ntp_result {
            embassy_futures::select::Either::First(result) => result.map_err(|e| {
                log::error!("NTP get_time failed: {:?}", e);
                e
            })?,
            embassy_futures::select::Either::Second(_) => {
                log::error!("NTP request timed out after 10 seconds");
                return Err(SntpcError::Sntc(sntpc::Error::Network));
            }
        };

        log::info!("NTP response received, timestamp: {}", ntp_result.seconds);

        let now = DateTime::from_timestamp(ntp_result.seconds as i64, 0).ok_or_else(|| {
            log::error!("Failed to parse NTP timestamp: {}", ntp_result.seconds);
            SntpcError::BadNtpResponse
        })?;
        let now = now.with_timezone(&GB);

        log::info!("Setting time to: {:?}", now);
        time.set_time(now).await;

        Ok(())
    }

    /// Try to resolve NTP server via DNS
    async fn try_dns_query(stack: Stack<'static>) -> Result<SocketAddr, SntpcError> {
        // Retry DNS query a few times since it might fail initially
        for attempt in 1..=3 {
            log::info!("DNS query attempt {} of 3 for {}", attempt, POOL_NTP_ADDR);
            match stack.dns_query(POOL_NTP_ADDR, DnsQueryType::A).await {
                Ok(mut addrs) => {
                    if let Some(addr) = addrs.pop() {
                        match addr {
                            embassy_net::IpAddress::Ipv4(ipv4) => {
                                let octets = *ipv4.as_octets();
                                let ipv4_addr = no_std_net::Ipv4Addr::new(
                                    octets[0], octets[1], octets[2], octets[3],
                                );
                                let sock_addr =
                                    SocketAddr::new(no_std_net::IpAddr::V4(ipv4_addr), 123);
                                log::info!("DNS resolved to: {:?}", sock_addr);
                                return Ok(sock_addr);
                            } // Currently only IPv4 is supported by embassy-net in this configuration
                        }
                    } else {
                        log::error!("DNS returned empty response");
                    }
                }
                Err(e) => {
                    log::error!("DNS query attempt {} failed: {:?}", attempt, e);
                    if attempt < 3 {
                        Timer::after_secs(2).await;
                    }
                }
            }
        }
        Err(SntpcError::DnsEmptyResponse)
    }
}
