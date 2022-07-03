use anyhow::Context;
use argh::FromArgs;

mod stadia;

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

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    // Parse arguments.
    let args = argh::from_env::<Args>();

    // Connect to ViGEm and create a X360 controller.
    let mut client = vigem::Vigem::new();

    client.connect().context("cannot connect to ViGEm")?;

    let mut target = vigem::Target::new(vigem::TargetType::Xbox360);

    client
        .target_add(&mut target)
        .context("cannot add Xbox 360 controller to ViGEm")?;

    // Create Stadia controller.
    #[derive(Debug)]
    struct Vibration {
        large_motor: u8,
        small_motor: u8,
    }

    let mut controller = stadia::Controller::new();
    let (mut tx_vibration, mut rx_vibration) = tokio::sync::mpsc::unbounded_channel();

    // Set notifications handler, which forwards vibrations.
    unsafe extern "C" fn handle_notification(
        _client: *mut vigem::raw::_VIGEM_CLIENT_T,
        _target: *mut vigem::raw::_VIGEM_TARGET_T,
        large_motor: u8,
        small_motor: u8,
        _led_number: u8,
        tx_vibration: *mut tokio::sync::mpsc::UnboundedSender<Vibration>,
    ) {
        let _ = (*tx_vibration).send(Vibration {
            large_motor,
            small_motor,
        });
    }

    client
        .x360_register_notification(&target, Some(handle_notification), &mut tx_vibration)
        .context("cannot register ViGEm vibration notification")?;

    // Run event loop.
    let mut was_assistant_pressed = false;
    let mut was_capture_pressed = false;

    loop {
        tokio::select! {
            // Stop on Ctrl-C.
            _ = tokio::signal::ctrl_c() => return Ok(()),

            // Forward reports from the Stadia controller to the ViGEm Xbox 360
            // virtual controller.
            report = controller.read_report() => {
                let report = report.context("cannot read controller report")?;

                target
                    .update(&report.vigem_report)
                    .context("cannot forward Stadia controller action to ViGEm")?;

                // Handle presses to the Assistant and Capture buttons.
                let (assistant_result, capture_result) = tokio::join!(
                    run_button_press(
                        args.shell.as_deref(),
                        report.is_assistant_pressed,
                        &mut was_assistant_pressed,
                        args.assistant_pressed.as_deref(),
                        args.assistant_released.as_deref(),
                    ),
                    run_button_press(
                        args.shell.as_deref(),
                        report.is_capture_pressed,
                        &mut was_capture_pressed,
                        args.capture_pressed.as_deref(),
                        args.capture_released.as_deref(),
                    ),
                );

                assistant_result.context("cannot run Assistant handler")?;
                capture_result.context("cannot run Capture handler")?;
            },

            // Forward vibrations from the ViGEm Xbox 360 virtual controller to
            // the Stadia controller.
            Some(Vibration { large_motor, small_motor }) = rx_vibration.recv() => {
                controller
                    .vibrate(large_motor, small_motor)
                    .context("cannot forward vibration to Stadia controller")?;
            },
        }
    }
}

/// Runs the command `if_pressed` if `currently_pressed` is true, and the
/// command `if_released` otherwise. Updates `previously_pressed` to
/// `currently_pressed`.
///
/// If `*previously_pressed == currently_pressed`, does not do anything.
async fn run_button_press(
    shell: Option<&str>,
    currently_pressed: bool,
    previously_pressed: &mut bool,
    if_pressed: Option<&str>,
    if_released: Option<&str>,
) -> anyhow::Result<()> {
    if *previously_pressed == currently_pressed {
        return Ok(());
    }

    *previously_pressed = currently_pressed;

    let command_to_run = match (currently_pressed, if_pressed, if_released) {
        (true, Some(cmd), _) | (false, _, Some(cmd)) if !cmd.is_empty() => cmd,
        _ => return Ok(()),
    };
    let shell = shell.unwrap_or("cmd");

    let mut child = tokio::process::Command::new(shell)
        .arg("/C")
        .arg(command_to_run)
        .spawn()
        .with_context(|| format!("cannot spawn command '{shell} /C {command_to_run:?}'"))?;

    if let Err(err) = child.wait().await {
        eprintln!("command '{shell} /C {command_to_run:?}' failed: {err}");
    }

    Ok(())
}
