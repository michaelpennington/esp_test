#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![feature(impl_trait_in_assoc_type)]

use defmt::{error, info};
use embassy_executor::Spawner;
use embassy_net::{Stack, StackResources};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use esp_hal::{
    clock::CpuClock,
    gpio::{Level, Output, OutputConfig},
    peripherals::RMT,
    rng::Rng,
    timer::timg::TimerGroup,
};
use esp_radio::wifi::{ClientConfig, ModeConfig, WifiController, WifiDevice, WifiStaState};
use my_esp_project::neopixel::{NeoPixelDriver, RGB};
use panic_rtt_target as _;
use picoserve::{
    AppBuilder, AppRouter, Router, Server, Timeouts,
    extract::Query,
    response::{File, IntoResponse},
    routing::{PathRouter, get, get_service},
};
use static_cell::{ConstStaticCell, StaticCell};

extern crate alloc;

const WIFI_SSID: &str = "wireless24!";
const WIFI_PASSWORD: &str = "Rusty007!";

static COLOR_SIGNAL: Signal<CriticalSectionRawMutex, RGB> = Signal::new();
static RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
static RADIO_INIT: StaticCell<esp_radio::Controller<'_>> = StaticCell::new();
static RX_BUFFER: ConstStaticCell<[u8; 1024]> = ConstStaticCell::new([0; 1024]);
static TX_BUFFER: ConstStaticCell<[u8; 1024]> = ConstStaticCell::new([0; 1024]);
static HTTP_BUFFER: ConstStaticCell<[u8; 2048]> = ConstStaticCell::new([0; 2048]);

// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    rtt_target::rtt_init_defmt!();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let rng = Rng::new();
    let seed = rng.random() as u64;

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 66320);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    info!("Embassy initialized!");

    let radio_init =
        RADIO_INIT.init(esp_radio::init().expect("Failed to initialize Wi-Fi/BLE controller"));
    let (wifi_controller, wifi_device) =
        esp_radio::wifi::new(radio_init, peripherals.WIFI, Default::default())
            .expect("Failed to initialize Wi-Fi controller");
    let config = embassy_net::Config::dhcpv4(Default::default());
    let resources = RESOURCES.init(StackResources::new());
    let (stack, runner) = embassy_net::new(wifi_device.sta, config, resources, seed);
    spawner.spawn(connection_task(wifi_controller)).unwrap();
    spawner.spawn(net_task(runner)).unwrap();
    spawner
        .spawn(web_task(
            stack,
            RX_BUFFER.take(),
            TX_BUFFER.take(),
            HTTP_BUFFER.take(),
        ))
        .unwrap();
    spawner
        .spawn(led(
            peripherals.RMT,
            Output::new(peripherals.GPIO8, Level::Low, OutputConfig::default()),
        ))
        .unwrap();

    loop {
        info!("Hello world!");
        Timer::after(Duration::from_secs(1)).await;
    }
}

struct App;

impl AppBuilder for App {
    type PathRouter = impl PathRouter;

    fn build_app(self) -> Router<Self::PathRouter> {
        Router::new()
            .route("/", get_service(File::html(include_str!("../index.html"))))
            .route("/color", get(set_color))
    }
}

async fn set_color(params: Query<ColorParams>) -> impl IntoResponse {
    if let Ok(color) = parse_hex_color(&params.v) {
        COLOR_SIGNAL.signal(color);
        "Color updated"
    } else {
        "Invalid color format"
    }
}

#[derive(serde::Deserialize)]
struct ColorParams {
    v: alloc::string::String,
}

#[embassy_executor::task]
async fn web_task(
    stack: Stack<'static>,
    rx_buffer: &'static mut [u8],
    tx_buffer: &'static mut [u8],
    http_buffer: &'static mut [u8],
) {
    stack.wait_config_up().await;
    info!(
        "Network is up! IP: {:?}",
        stack.config_v4().unwrap().address
    );

    let app = picoserve::make_static!(AppRouter<App>, App.build_app());

    let config = picoserve::Config::new(Timeouts {
        start_read_request: Some(Duration::from_secs(5)),
        persistent_start_read_request: None,
        read_request: Some(Duration::from_secs(1)),
        write: Some(Duration::from_secs(1)),
    })
    .keep_connection_alive();
    let server = Server::new(app, &config, http_buffer);

    server
        .listen_and_serve(1, stack, 3, rx_buffer, tx_buffer)
        .await;
}

fn parse_hex_color(hex: &str) -> Result<RGB, ()> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return Err(());
    }
    let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| ())?;
    let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| ())?;
    let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| ())?;
    Ok(RGB { r, g, b })
}

#[embassy_executor::task]
async fn led(rmt: RMT<'static>, pin: Output<'static>) {
    let mut neopixel = NeoPixelDriver::new(rmt, pin).unwrap();
    let color = RGB::default();
    neopixel.set_led(color).await.unwrap();
    loop {
        let new_color = COLOR_SIGNAL.wait().await;
        neopixel.set_led(new_color).await.unwrap();
        info!("LED updated: {:?}", new_color);
    }
}

#[embassy_executor::task]
async fn connection_task(mut controller: WifiController<'static>) {
    info!("Starting connection task");
    loop {
        if esp_radio::wifi::sta_state() == WifiStaState::Connected {
            Timer::after(Duration::from_secs(5)).await;
            continue;
        }

        info!("Scanning/Connecting to WiFi...");
        let client_config = ClientConfig::default()
            .with_ssid(WIFI_SSID.into())
            .with_password(WIFI_PASSWORD.into());

        match controller.set_config(&ModeConfig::Client(client_config)) {
            Ok(_) => info!("Configuration set"),
            Err(e) => error!("Failed to set config: {:?}", e),
        }

        match controller.start_async().await {
            Ok(_) => info!("Wifi started"),
            Err(e) => error!("Failed to start wifi: {:?}", e),
        }

        match controller.connect_async().await {
            Ok(_) => info!("Wifi connected!"),
            Err(e) => {
                error!("Failed to connect: {:?}", e);
                Timer::after(Duration::from_secs(2)).await;
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}
