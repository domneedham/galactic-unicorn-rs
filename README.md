# Galactic Unicorn App

A smart clock controlled via MQTT with support for home assistant.

## Prerequisites

You will need to create a `config.rs` in the `src/` folder.

Copy the following content and adjust to your requirements. All keys must be present and configured correctly.

```rust
// wifi details
pub const WIFI_NETWORK: &str = "Your-Wifi-SSID";
pub const WIFI_PASSWORD: &str = "Your-Wifi-Password";

// device IP config
// static IP is currently required
pub const IP_A1: u8 = 192;
pub const IP_A2: u8 = 168;
pub const IP_A3: u8 = 1;
pub const IP_A4: u8 = 10;
pub const PREFIX_LENGTH: u8 = 24;

// router IP config
pub const GW_A1: u8 = 192;
pub const GW_A2: u8 = 168;
pub const GW_A3: u8 = 1;
pub const GW_A4: u8 = 254;

// mqtt config
pub const MQTT_BROKER_A1: u8 = 192;
pub const MQTT_BROKER_A2: u8 = 168;
pub const MQTT_BROKER_A3: u8 = 1;
pub const MQTT_BROKER_A4: u8 = 253;
pub const MQTT_BROKER_PORT: u16 = 1883;
pub const MQTT_USERNAME: &str = "";
pub const MQTT_PASSWORD: &str = "";

// the base mqtt topic the device will send/listen to
pub const BASE_MQTT_TOPIC: &str = "galactic_unicorn";

// the device id
pub const DEVICE_ID: &'static str = "galactic_unicorn";

// home assistant base mqtt topic
pub const HASS_BASE_MQTT_TOPIC: &'static str = "homeassistant";

```

## Roadmap

- [x] Generic clock
- [x] Display effects
- [x] Button controlled
- [x] MQTT controlled
- [x] Queue and display messages from MQTT
- [x] Home assistant discovery
- [x] Network resiliency
- [ ] Good documentation
- [ ] Web configuration portal
- [ ] Feature disablement
- [ ] Saving last known config for reboot
- [ ] More effects / animations
- [ ] Speaker usage
- [ ] Utilise D button

## Known Issues

- MQTT server going offline causes a panic. This is a 3rd party dependency issue I need to further investigate.

## Development Requirements

- The standard Rust tooling (cargo, rustup) which you can install from https://rustup.rs/

- Rust nightly

- Toolchain support for the cortex-m0+ processors in the rp2040 (thumbv6m-none-eabi)

```sh
rustup install nightly
rustup +nightly target add thumbv6m-none-eabi
cargo +nightly cargo install elf2uf2-rs
```

## Running

For a debug build

```sh
cargo run
```

For a release build

```sh
cargo run --release
```

## Contributing

Contributions are what make the open source community such an amazing place to be learn, inspire, and create. Any contributions you make are **greatly appreciated**.

The steps are:

1. Fork the Project by clicking the 'Fork' button at the top of the page.
2. Create your Feature Branch (`git checkout -b features/AmazingFeature`)
3. Make some changes to the code or documentation.
4. Commit your Changes (`git commit -m 'Add some AmazingFeature'`)
5. Push to the Feature Branch (`git push origin features/AmazingFeature`)
6. Create a new pull request
7. An admin will review the Pull Request and discuss any changes that may be required.
8. Once everyone is happy, the Pull Request can be merged by an admin, and your work is part of our project!

> There are linting policies on the project. Please use `cargo clippy` before submitting a pull request and fix _all_ warnings. The automated builds will fail if a warning is generated.

## Code of Conduct

See the [code of conduct](CODE_OF_CONDUCT.md).

## License

The contents of this repository are dual-licensed under the _MIT OR Apache
2.0_ License. That means you can chose either the MIT licence or the
Apache-2.0 licence when you re-use this code. See `MIT` or `APACHE2.0` for more
information on each specific licence.

Any submissions to this project (e.g. as Pull Requests) must be made available
under these terms.
