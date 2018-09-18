use std::env;
use std::fs::File;
use std::io::prelude::*;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chan::Sender;
use regex::Regex;
use uuid::Uuid;

use crate::block::{Block, ConfigBlock};
use crate::blocks::dbus::stdintf::org_freedesktop_dbus::Properties;
use crate::blocks::dbus::{arg, Connection, ConnectionItem};
use crate::config::Config;
use crate::errors::*;
use crate::input::I3BarEvent;
use crate::scheduler::Task;
use crate::util;
use crate::widget::I3BarWidget;
use crate::widgets::text::TextWidget;

pub struct IBus {
    id: String,
    text: TextWidget,
    engine: Arc<Mutex<String>>,
}

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct IBusConfig {
    // TODO: Implement this.
    /// Set to display engine name as the two letter country abbreviation, e.g. "jp".
    #[serde(default = "IBusConfig::default_abbreviate")]
    pub as_icon: bool,
}

impl IBusConfig {
    fn default_abbreviate() -> bool {
        true
    }
}

impl ConfigBlock for IBus {
    type Config = IBusConfig;

    fn new(_block_config: Self::Config, config: Config, send: Sender<Task>) -> Result<Self> {
        let id: String = Uuid::new_v4().simple().to_string();
        let id_copy = id.clone();

        let ibus_address = get_ibus_address()?;
        let c = Connection::open_private(&ibus_address).block_error(
            "ibus",
            &format!("Failed to establish D-Bus connection to {}", ibus_address),
        )?;
        let p = c.with_path("org.freedesktop.IBus", "/org/freedesktop/IBus", 5000);
        let info: arg::Variant<Box<arg::RefArg>> = p
            .get("org.freedesktop.IBus", "GlobalEngine")
            .block_error("ibus", "Failed to query IBus")?;

        // `info` should contain something containing an array with the contents as such:
        // [name, longname, description, language, license, author, icon, layout, layout_variant, layout_option, rank, hotkeys, symbol, setup, version, textdomain, icon_prop_key]
        // Refer to: https://github.com/ibus/ibus/blob/7cef5bf572596361bc502e8fa917569676a80372/src/ibusenginedesc.c
        // e.g.                   name           longname        description     language
        // ["IBusEngineDesc", {}, "xkb:us::eng", "English (US)", "English (US)", "en", "GPL", "Peng Huang <shawn.p.huang@gmail.com>", "ibus-keyboard", "us", 99, "", "", "", "", "", "", "", ""]
        //                         ↑ We will use this element (name) as it is what GlobalEngineChanged signal returns.
        let current_engine = info
            .0
            .as_iter()
            .block_error("ibus", "Failed to parse D-Bus message (step 1)")?
            .nth(2)
            .block_error("ibus", "Failed to parse D-Bus message (step 2)")?
            .as_str()
            .unwrap_or("??");
        let engine_original = Arc::new(Mutex::new(String::from(current_engine)));

        let engine = engine_original.clone();
        thread::spawn(move || {
            let c = Connection::open_private(&ibus_address)
                .expect("Failed to establish D-Bus connection in thread");
            c.add_match("interface='org.freedesktop.IBus',member='GlobalEngineChanged'")
                .expect("Failed to add D-Bus message rule - has IBus interface changed?");
            loop {
                for ci in c.iter(100000) {
                    if let Some(engine_name) = parse_msg(&ci) {
                        let mut engine = engine_original.lock().unwrap();
                        *engine = engine_name.to_string();
                        // Tell block to update now.
                        send.send(Task {
                            id: id.clone(),
                            update_time: Instant::now(),
                        });
                    };
                }
            }
        });

        Ok(IBus {
            id: id_copy,
            text: TextWidget::new(config.clone()).with_text("IBus"),
            engine,
        })
    }
}

impl Block for IBus {
    fn id(&self) -> &str {
        &self.id
    }

    // Updates the internal state of the block.
    fn update(&mut self) -> Result<Option<Duration>> {
        let engine = (*self
            .engine
            .lock()
            .block_error("ibus", "failed to acquire lock")?)
        .clone();
        self.text.set_text(engine);
        Ok(None)
    }

    // Returns the view of the block, comprised of widgets.
    fn view(&self) -> Vec<&I3BarWidget> {
        vec![&self.text]
    }

    // This function is called on every block for every click.
    // TODO: Filter events by using the event.name property,
    // and use to switch between input engines?
    fn click(&mut self, _: &I3BarEvent) -> Result<()> {
        Ok(())
    }
}

fn parse_msg(ci: &ConnectionItem) -> Option<&str> {
    let m = if let &ConnectionItem::Signal(ref s) = ci {
        s
    } else {
        return None;
    };
    if &*m.interface().unwrap() != "org.freedesktop.IBus" {
        return None;
    };
    if &*m.member().unwrap() != "GlobalEngineChanged" {
        return None;
    };
    let engine = m.get1::<&str>();
    engine
}

// Gets the address being used by the currently running ibus daemon.
//
// By default ibus will write the address to `$XDG_CONFIG_HOME/ibus/bus/aaa-bbb-ccc`
// where aaa = dbus machine id, usually found at /etc/machine-id
//       bbb = hostname - seems to be "unix" in most cases [see L99 of reference]
//       ccc = display number from $DISPLAY
// Refer to: https://github.com/ibus/ibus/blob/7cef5bf572596361bc502e8fa917569676a80372/src/ibusshare.c
//
// Example file contents:
// ```
// # This file is created by ibus-daemon, please do not modify it
// IBUS_ADDRESS=unix:abstract=/tmp/dbus-8EeieDfT,guid=7542d73dce451c2461a044e24bc131f4
// IBUS_DAEMON_PID=11140
// ```
fn get_ibus_address() -> Result<String> {
    // TODO: Check IBUS_ADDRESS variable, as it seems it can be manually set too.

    // TODO: Don't fail if $XDG_CONFIG_HOME is not set. 
    // Next try $HOME/.config, then only error if that $HOME is not set.
    let config_dir = env::var("XDG_CONFIG_HOME")
        .block_error("ibus", "$XDG_CONFIG_HOME not set")?;

    // TODO: Check /var/lib/dbus/machine-id if /etc/machine-id fails
    let mut f = File::open("/etc/machine-id")
        .block_error("ibus", "Could not open /etc/machine-id")?;
    let mut machine_id = String::new();
    f.read_to_string(&mut machine_id)
        .block_error("ibus", "Something went wrong reading /etc/machine-id")?;
    let machine_id = machine_id.trim();

    // On sway, $DISPLAY is only set by programs requiring xwayland, such as ibus (GTK2).
    // ibus-daemon can be autostarted by sway (via an entry in config file), however since
    // the bar is executed first, $DISPLAY will not yet be set at the time this code runs.
    // Hence on sway you will need to reload the bar once after login to get the block to work.
    let display_var = env::var("DISPLAY")
        .block_error("ibus", "$DISPLAY not set. Try restarting bar if on sway")?;
    let re = Regex::new(r"^:(\d{1})$").unwrap(); // valid regex expression will not cause panic
    let cap = re.captures(&display_var)
        .block_error("ibus", "Failed to extract display number from $DISPLAY")?;
    let display_number = &cap[1].to_string();

    let hostname = String::from("unix");

    let ibus_socket_path = format!("{}/ibus/bus/{}-{}-{}", config_dir, machine_id, hostname, display_number);
    let mut f = File::open(&ibus_socket_path)
        .block_error("ibus", &format!("Could not open {}", ibus_socket_path))?;
    let mut ibus_address = String::new();
    f.read_to_string(&mut ibus_address)
        .block_error("ibus", &format!("Error reading contents of {}", ibus_socket_path))?;
    let re = Regex::new(r"IBUS_ADDRESS=(.*),guid").unwrap(); // valid regex expression will not cause panic
    let cap = re.captures(&ibus_address)
        .block_error("ibus", &format!("Failed to extract address out of {}", ibus_address))?;
    let ibus_address = &cap[1];

    Ok(
        ibus_address.to_string()
    )
}
