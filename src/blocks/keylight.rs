//! A block for displaying the backlight of the keyboard.
//!
//! This module contains the [`Keylight`](./struct.Keylight.html) block, which
//! can display the keylight level of brightness of the keyboard (any vendor). Brightness
//! levels are read from the `sysfs` filesystem, so this block
//! does not depend on any specific binary (and thus it works on Wayland).

use std::fs::OpenOptions;
use std::time::Duration;

use crossbeam_channel::Sender;
use serde_derive::Deserialize;
use uuid::Uuid;

use crate::blocks::{Block, ConfigBlock, Update};
use crate::config::Config;
use crate::de::deserialize_duration;
use crate::errors::*;
use crate::input::I3BarEvent;
use crate::scheduler::Task;
use crate::widget::I3BarWidget;
use crate::widgets::text::TextWidget;

/// Read a brightness value from the given path.
fn read_brightness(device_file: &Path) -> Result<u16> {
    let mut file = OpenOptions::new()
        .read(true)
        .open(device_file)
        .block_error("keylight", "Failed to open brightness file")?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .block_error("keylight", "Failed to read brightness file")?;
    // Removes trailing newline.
    content.pop();
    content
        .parse::<u16>()
        .block_error("keylight", "Failed to read value from brightness file")
}

pub struct Keylight {
    text: TextWidget,
    id: String,
    update_interval: Duration,

    //useful, but optional
    #[allow(dead_code)]
    config: Config,
    #[allow(dead_code)]
    tx_update_request: Sender<Task>,
}

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct KeylightConfig {
    /// Update interval in seconds
    #[serde(
        default = "KeylightConfig::default_interval",
        deserialize_with = "deserialize_duration"
    )]
    pub interval: Duration,
}

impl KeylightConfig {
    fn default_interval() -> Duration {
        Duration::from_secs(5)
    }
}

impl ConfigBlock for Keylight {
    type Config = KeylightConfig;

    fn new(
        block_config: Self::Config,
        config: Config,
        tx_update_request: Sender<Task>,
    ) -> Result<Self> {
        Ok(Keylight {
            id: Uuid::new_v4().to_simple().to_string(),
            update_interval: block_config.interval,
            text: TextWidget::new(config.clone()).with_text("Keylight"),
            tx_update_request,
            config,
        })
    }
}

impl Block for Keylight {
    fn update(&mut self) -> Result<Option<Update>> {
        Ok(Some(self.update_interval.into()))
    }

    fn view(&self) -> Vec<&dyn I3BarWidget> {
        vec![&self.text]
    }

    fn click(&mut self, _: &I3BarEvent) -> Result<()> {
        Ok(())
    }

    fn id(&self) -> &str {
        &self.id
    }
}
