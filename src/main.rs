//! Matter/Thread Christmas Garland for ESP32-C6
//!
//! Controls a garland via MOSFET on GPIO18 using Matter protocol over Thread.
//! BLE is used for commissioning.

#![allow(unexpected_cfgs)]
#![recursion_limit = "256"]

fn main() -> Result<(), anyhow::Error> {
    #[cfg(any(esp32c6, esp32h2))]
    {
        garland::main()
    }

    #[cfg(not(any(esp32c6, esp32h2)))]
    panic!("This firmware is only supported on ESP32-C6 and ESP32-H2 chips.");
}

#[cfg(any(esp32c6, esp32h2))]
mod garland {
    use core::cell::Cell;
    use core::pin::pin;

    use alloc::sync::Arc;

    use esp_idf_matter::init_async_io;
    use esp_idf_matter::matter::dm::clusters::decl::on_off as on_off_cluster;
    use esp_idf_matter::matter::dm::clusters::desc::{self, ClusterHandler as _, DescHandler};
    use esp_idf_matter::matter::dm::clusters::on_off::{
        self, EffectVariantEnum, OnOffHandler, OnOffHooks, StartUpOnOffEnum,
    };
    use esp_idf_matter::matter::dm::devices::test::{TEST_DEV_ATT, TEST_DEV_COMM, TEST_DEV_DET};
    use esp_idf_matter::matter::dm::devices::DEV_TYPE_ON_OFF_LIGHT;
    use esp_idf_matter::matter::dm::{Async, Cluster, Dataver, EmptyHandler, Endpoint, EpClMatcher, Node};
    use esp_idf_matter::matter::error::Error;
    use esp_idf_matter::matter::tlv::Nullable;
    use esp_idf_matter::matter::utils::init::InitMaybeUninit;
    use esp_idf_matter::matter::{clusters, devices, with};
    use esp_idf_matter::persist::EspKvBlobStore;
    use esp_idf_matter::wireless::{EspMatterThread, EspThreadMatterStack};

    use esp_idf_svc::bt::reduce_bt_memory;
    use esp_idf_svc::eventloop::EspSystemEventLoop;
    use esp_idf_svc::hal::peripherals::Peripherals;
    use esp_idf_svc::hal::task::block_on;
    use esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration;
    use esp_idf_svc::io::vfs::MountedEventfs;
    use esp_idf_svc::nvs::EspDefaultNvsPartition;
    use esp_idf_svc::sys::{gpio_config, gpio_config_t, gpio_mode_t_GPIO_MODE_OUTPUT, gpio_set_level};

    use log::{error, info};

    use static_cell::StaticCell;

    extern crate alloc;

    const STACK_SIZE: usize = 20 * 1024;
    const BUMP_SIZE: usize = 13500;
    const GPIO_NUM: i32 = 18;
    const LIGHT_ENDPOINT_ID: u16 = 1;

    pub struct GarlandController {
        state: Cell<bool>,
    }

    impl GarlandController {
        pub fn new() -> Self {
            Self {
                state: Cell::new(false),
            }
        }
    }

    impl OnOffHooks for GarlandController {
        const CLUSTER: Cluster<'static> = on_off_cluster::FULL_CLUSTER
            .with_revision(6)
            .with_attrs(with!(required; on_off_cluster::AttributeId::OnOff))
            .with_cmds(with!(
                on_off_cluster::CommandId::Off
                    | on_off_cluster::CommandId::On
                    | on_off_cluster::CommandId::Toggle
            ));

        fn on_off(&self) -> bool {
            self.state.get()
        }

        fn set_on_off(&self, on: bool) {
            self.state.set(on);
            let level = if on { 1 } else { 0 };
            let ret = unsafe { gpio_set_level(GPIO_NUM, level) };
            if ret == 0 {
                info!("Garland: {}", if on { "ON" } else { "OFF" });
            } else {
                error!("Garland: {} FAILED: {}", if on { "ON" } else { "OFF" }, ret);
            }
        }

        fn start_up_on_off(&self) -> Nullable<StartUpOnOffEnum> {
            Nullable::some(StartUpOnOffEnum::Off)
        }

        fn set_start_up_on_off(&self, _value: Nullable<StartUpOnOffEnum>) -> Result<(), Error> {
            Ok(())
        }

        async fn handle_off_with_effect(&self, _effect: EffectVariantEnum) {
            self.set_on_off(false);
        }
    }

    static MATTER_STACK: StaticCell<EspThreadMatterStack<BUMP_SIZE, ()>> = StaticCell::new();

    const NODE: Node = Node {
        id: 0,
        endpoints: &[
            EspThreadMatterStack::<0, ()>::root_endpoint(),
            Endpoint {
                id: LIGHT_ENDPOINT_ID,
                device_types: devices!(DEV_TYPE_ON_OFF_LIGHT),
                clusters: clusters!(DescHandler::CLUSTER, GarlandController::CLUSTER),
            },
        ],
    };

    pub fn main() -> Result<(), anyhow::Error> {
        esp_idf_svc::log::init_from_env();

        info!("Starting Matter Garland...");

        ThreadSpawnConfiguration::set(&ThreadSpawnConfiguration {
            name: Some(c"matter"),
            ..Default::default()
        })?;

        let thread = std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn(run)
            .unwrap();

        thread.join().unwrap()
    }

    #[inline(never)]
    #[cold]
    fn run() -> Result<(), anyhow::Error> {
        let result = block_on(matter());

        if let Err(e) = &result {
            error!("Matter aborted execution with error: {e:?}");
        } else {
            info!("Matter finished execution successfully");
        }

        result
    }

    async fn matter() -> Result<(), anyhow::Error> {
        let stack = MATTER_STACK
            .uninit()
            .init_with(EspThreadMatterStack::init_default(
                &TEST_DEV_DET,
                TEST_DEV_COMM,
                &TEST_DEV_ATT,
            ));

        info!("Matter initialized");

        let sysloop = EspSystemEventLoop::take()?;
        let nvs = EspDefaultNvsPartition::take()?;
        let mut peripherals = Peripherals::take()?;

        let mounted_event_fs = Arc::new(MountedEventfs::mount(6)?);
        init_async_io(mounted_event_fs.clone())?;

        reduce_bt_memory(unsafe { peripherals.modem.reborrow() })?;

        info!("Basics initialized");

        // Initialize GPIO18 for MOSFET-controlled garland
        let io_conf = gpio_config_t {
            pin_bit_mask: 1u64 << GPIO_NUM,
            mode: gpio_mode_t_GPIO_MODE_OUTPUT,
            pull_up_en: 0,
            pull_down_en: 0,
            intr_type: 0,
        };

        unsafe {
            gpio_config(&io_conf);
        }

        let garland = GarlandController::new();
        info!("GPIO18 initialized for garland control");

        let on_off = OnOffHandler::new_standalone(
            Dataver::new_rand(stack.matter().rand()),
            LIGHT_ENDPOINT_ID,
            garland,
        );

        let handler = EmptyHandler
            .chain(
                EpClMatcher::new(
                    Some(LIGHT_ENDPOINT_ID),
                    Some(GarlandController::CLUSTER.id),
                ),
                on_off::HandlerAsyncAdaptor(&on_off),
            )
            .chain(
                EpClMatcher::new(Some(LIGHT_ENDPOINT_ID), Some(DescHandler::CLUSTER.id)),
                Async(desc::DescHandler::new(Dataver::new_rand(stack.matter().rand())).adapt()),
            );

        info!("Handler initialized");

        let kvs = EspKvBlobStore::new_default(nvs.clone())?;
        let persist = stack
            .create_persist_with_comm_window(kvs)
            .await?;

        let matter = pin!(stack.run_coex(
            EspMatterThread::new(peripherals.modem, sysloop, nvs, mounted_event_fs, stack),
            &persist,
            (NODE, handler),
            (),
        ));

        info!("About to run Matter");

        matter.await?;

        Ok(())
    }
}
