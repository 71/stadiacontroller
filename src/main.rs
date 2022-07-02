use std::{ffi::c_void, process::Command};

use anyhow::Context;
use argh::FromArgs;
use tokio::sync::watch;
use vigem::XUSBReport;

mod stadia;

use stadia::Controller;

#[derive(FromArgs)]
/// Emulate an Xbox 360 controller using a Stadia controller.
struct Args {
    /// command to run when the Assistant button is pressed
    #[argh(option)]
    assistant_pressed: Option<String>,

    /// command to run when the Assistant button is released
    #[argh(option)]
    assistant_released: Option<String>,

    /// command to run when the Capture button is pressed
    #[argh(option)]
    capture_pressed: Option<String>,

    /// command to run when the Capture button is released
    #[argh(option)]
    capture_released: Option<String>,

    /// shell to use to run the commands
    #[argh(option)]
    shell: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse arguments.
    let args = argh::from_env();

    // Create channels for communication.
    let (tx_vibration, rx_vibration) = watch::channel(Vibration::default());
    let (tx_report, rx_report) = watch::channel(XUSBReport::default());

    // Spawn loops.
    let vigem_loop = vigem_loop(rx_report, tx_vibration);
    let stadia_loop = stadia_loop(args, rx_vibration, tx_report);

    tokio::try_join!(vigem_loop, stadia_loop)?;

    Ok(())
}

async fn vigem_loop(
    mut rx_report: watch::Receiver<XUSBReport>,
    mut tx_vibration: watch::Sender<Vibration>,
) -> anyhow::Result<()> {
    // Create client and target controller.
    let mut client = vigem::Vigem::new();

    client.connect().context("connecting to vigem")?;

    let mut target = vigem::Target::new(vigem::TargetType::Xbox360);

    client
        .target_add(&mut target)
        .context("adding xbox 360 target to vigem")?;

    // Set notifications handler, which forwards vibrations.
    unsafe extern "C" fn handle_notification(
        _client: *mut vigem::raw::_VIGEM_CLIENT_T,
        _target: *mut vigem::raw::_VIGEM_TARGET_T,
        large_motor: u8,
        small_motor: u8,
        _led_number: u8,
        tx_vibration: *mut watch::Sender<Vibration>,
    ) {
        let vibration = Vibration {
            large_motor,
            small_motor,
        };

        (*tx_vibration).send(vibration).unwrap();
    }

    client.x360_register_notification(&target, Some(handle_notification), &mut tx_vibration)?;

    // Forward Stadia reports to ViGEm.
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),

            _ = rx_report.changed() => {
                target.update(&*rx_report.borrow()).context("updating vigem controller")?;
            },
        }
    }
}

async fn stadia_loop(
    args: Args,
    mut rx_vibration: watch::Receiver<Vibration>,
    tx_report: watch::Sender<XUSBReport>,
) -> anyhow::Result<()> {
    let mut controller = Controller::new();
    let mut was_assistant_pressed = false;
    let mut was_capture_pressed = false;

    loop {
        tokio::select! {
            // Stop on Ctrl-C.
            _ = tokio::signal::ctrl_c() => return Ok(()),

            // Receive reports and send them to ViGEm.
            report = controller.read_report() => {
                let report = report.context("reading controller report")?;

                tx_report.send(report.vigem_report)?;

                for (
                    previously_pressed,
                    currently_pressed,
                    command_if_pressed,
                    command_if_released,
                ) in [
                    (
                        &mut was_assistant_pressed,
                        report.is_assistant_pressed,
                        args.assistant_pressed.as_deref(),
                        args.assistant_released.as_deref(),
                    ),
                    (
                        &mut was_capture_pressed,
                        report.is_capture_pressed,
                        args.capture_pressed.as_deref(),
                        args.capture_released.as_deref(),
                    ),
                ] {
                    if *previously_pressed == currently_pressed {
                        continue;
                    }

                    *previously_pressed = currently_pressed;

                    run_button_press(
                        args.shell.as_deref(),
                        currently_pressed,
                        command_if_pressed,
                        command_if_released,
                    )?;
                }
            },

            // Forward ViGEm vibrations to the Stadia controller.
            _ = rx_vibration.changed() => {
                let vibration = rx_vibration.borrow();

                controller.vibrate(vibration.large_motor, vibration.small_motor).context("vibrating controller")?;
            },
        }
    }
}

fn run_button_press(
    shell: Option<&str>,
    is_pressed: bool,
    if_pressed: Option<&str>,
    if_released: Option<&str>,
) -> anyhow::Result<()> {
    let command_to_run = match (is_pressed, if_pressed, if_released) {
        (true, Some(cmd), _) | (false, _, Some(cmd)) if !cmd.is_empty() => cmd,
        _ => return Ok(()),
    };
    let shell = shell.unwrap_or("cmd");

    let mut child = Command::new(shell)
        .arg("/C")
        .arg(command_to_run)
        .spawn()
        .context("starting subprocess")?;
    let command_str = format!("{shell} /C {command_to_run:?}");

    tokio::spawn(async move {
        if let Err(err) = child.wait() {
            eprintln!("command '{command_str}' failed: {err}");
        }
    });

    Ok(())
}

#[derive(Clone, Copy, Default, Debug)]
struct Vibration {
    large_motor: u8,
    small_motor: u8,
}
