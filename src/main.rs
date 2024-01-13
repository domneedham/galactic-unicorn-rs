//! Galactic unicorn application.

#![no_std]
#![no_main]

mod config;

use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::Ipv4Address;
use embassy_net::Ipv4Cidr;
use embassy_net::Stack;
use embassy_net::StackResources;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::Input;
use embassy_rp::gpio::Level;
use embassy_rp::gpio::Output;
use embassy_rp::gpio::Pull;
use embassy_rp::peripherals::DMA_CH1;
use embassy_rp::peripherals::PIN_23;
use embassy_rp::peripherals::PIN_25;
use embassy_rp::peripherals::PIO1;
use embassy_rp::pio::InterruptHandler;
use embassy_rp::pio::Pio;
use embassy_time::Duration;
use embassy_time::Timer;
use heapless::Vec;

use cyw43_pio::PioSpi;

use defmt_rtt as _;
use panic_halt as _;
use rust_mqtt::client::client::MqttClient;
use rust_mqtt::utils::rng_generator::CountingRng;
use static_cell::StaticCell;

use embedded_graphics::mono_font::{ascii::FONT_5X8, MonoTextStyle};
use embedded_graphics::text::Text;
use embedded_graphics::Drawable;
use embedded_graphics_core::pixelcolor::RgbColor;
use embedded_graphics_core::{pixelcolor::Rgb888, prelude::Point};

use rust_mqtt::client::client_config::ClientConfig;
use rust_mqtt::packet::v5::reason_codes::ReasonCode;

use unicorn_graphics::UnicornGraphics;

use galactic_unicorn_embassy::buttons::UnicornButtons;
use galactic_unicorn_embassy::pins::{UnicornButtonPins, UnicornDisplayPins, UnicornPins};
use galactic_unicorn_embassy::GalacticUnicorn;
use galactic_unicorn_embassy::{HEIGHT, WIDTH};

use crate::config::*;

bind_interrupts!(struct Irqs {
    PIO1_IRQ_0 => InterruptHandler<PIO1>;
});

#[embassy_executor::task]
async fn wifi_task(
    runner: cyw43::Runner<
        'static,
        Output<'static, PIN_23>,
        PioSpi<'static, PIN_25, PIO1, 0, DMA_CH1>,
    >,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<cyw43::NetDriver<'static>>) -> ! {
    stack.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let fw = include_bytes!("../cyw43-firmware/43439A0.bin");
    let clm = include_bytes!("../cyw43-firmware/43439A0_clm.bin");

    let unipins = UnicornPins {
        display_pins: UnicornDisplayPins {
            column_clock: p.PIN_13,
            column_data: p.PIN_14,
            column_latch: p.PIN_15,
            column_blank: p.PIN_16,
            row_bit_0: p.PIN_17,
            row_bit_1: p.PIN_18,
            row_bit_2: p.PIN_19,
            row_bit_3: p.PIN_20,
        },

        button_pins: UnicornButtonPins {
            switch_a: Input::new(p.PIN_0, Pull::Up),
            switch_b: Input::new(p.PIN_1, Pull::Up),
            switch_c: Input::new(p.PIN_3, Pull::Up),
            switch_d: Input::new(p.PIN_6, Pull::Up),
            brightness_up: Input::new(p.PIN_21, Pull::Up),
            brightness_down: Input::new(p.PIN_26, Pull::Up),
            volume_up: Input::new(p.PIN_7, Pull::Up),
            volume_down: Input::new(p.PIN_8, Pull::Up),
            sleep: Input::new(p.PIN_27, Pull::Up),
        },
    };

    let mut gu = GalacticUnicorn::new(p.PIO0, unipins, p.DMA_CH0);

    let style = MonoTextStyle::new(&FONT_5X8, Rgb888::RED);
    let mut graphics = UnicornGraphics::<WIDTH, HEIGHT>::new();

    // wifi
    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO1, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        p.DMA_CH1,
    );
    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
    spawner.spawn(wifi_task(runner)).unwrap();

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    let config = embassy_net::Config::ipv4_static(embassy_net::StaticConfigV4 {
        address: Ipv4Cidr::new(Ipv4Address::new(IP_A1, IP_A2, IP_A3, IP_A4), PREFIX_LENGTH),
        dns_servers: Vec::new(),
        gateway: Some(Ipv4Address::new(GW_A1, GW_A2, GW_A3, GW_A4)),
    });
    // Generate random seed
    let seed = 0x0123_4567_89ab_cdef; // chosen by fair dice roll. guarenteed to be random.

    // Init network stack
    static STACK: StaticCell<Stack<cyw43::NetDriver<'static>>> = StaticCell::new();
    static RESOURCES: StaticCell<StackResources<2>> = StaticCell::new();
    let stack = &*STACK.init(Stack::new(
        net_device,
        config,
        RESOURCES.init(StackResources::<2>::new()),
        seed,
    ));

    spawner.spawn(net_task(stack)).unwrap();

    loop {
        match control.join_wpa2(WIFI_NETWORK, WIFI_PASSWORD).await {
            Ok(_) => break,
            Err(_) => {}
        }
    }

    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];
    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(Duration::from_secs(10)));
    let host_addr = embassy_net::Ipv4Address::new(192, 168, 1, 20);
    socket.connect((host_addr, 1883)).await;

    let mut config = ClientConfig::new(
        rust_mqtt::client::client_config::MqttVersion::MQTTv5,
        CountingRng(20000),
    );
    config.add_max_subscribe_qos(rust_mqtt::packet::v5::publish_packet::QualityOfService::QoS1);
    config.add_client_id("client");
    // config.add_username(USERNAME);
    // config.add_password(PASSWORD);
    config.max_packet_size = 100;
    let mut recv_buffer = [0; 80];
    let mut write_buffer = [0; 80];

    let mut client =
        MqttClient::<_, 5, _>::new(socket, &mut write_buffer, 80, &mut recv_buffer, 80, config);

    client.connect_to_broker().await.unwrap();

    // keep track of scroll position
    let mut x: i32 = -53;

    let mut message = "Welcome to Galactic Unicorn!";

    loop {
        Timer::after_millis(12).await;

        let width = message.len() * style.font.character_size.width as usize;
        x += 1;
        if x > width as i32 {
            x = -53;

            let res = client
                .send_message(
                    "hello",
                    b"Hello from the Galactic Unicorn!",
                    rust_mqtt::packet::v5::publish_packet::QualityOfService::QoS0,
                    true,
                )
                .await;

            match res {
                Ok(_) => message = "Success",
                Err(_) => message = "Failure",
            }
        }

        graphics.clear_all();
        Text::new(message, Point::new((0 - x) as i32, 7), style)
            .draw(&mut graphics)
            .unwrap();
        gu.update_and_draw(&graphics).await;

        if gu.is_button_pressed(UnicornButtons::BrightnessUp) {
            gu.increase_brightness(1);
        }

        if gu.is_button_pressed(UnicornButtons::BrightnessDown) {
            gu.decrease_brightness(1);
        }
    }
}
