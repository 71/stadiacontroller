package main

import (
	"errors"
	"flag"
	"fmt"
	"log"
	"os/exec"
	"time"

	"github.com/71/stadiacontroller"
)

var (
	shell = flag.String("shell", "pwsh", "a path to the shell to execute for commands")

	onCapturePressed    = flag.String("capture-pressed", "", "a command to run when the Capture button is pressed")
	onCaptureReleased   = flag.String("capture-released", "", "a command to run when the Capture button is released")
	onAssistantPressed  = flag.String("assistant-pressed", "", "a command to run when the Assistant button is pressed")
	onAssistantReleased = flag.String("assistant-released", "", "a command to run when the Assistant button is released")
)

func main() {
	flag.Parse()

	err := run()

	if err != nil {
		log.Fatal(err)
	}
}

func run() error {
	controller := stadiacontroller.NewStadiaController()

	defer controller.Close()

	emulator, err := stadiacontroller.NewEmulator(func(vibration stadiacontroller.Vibration) {
		controller.Vibrate(vibration.LargeMotor, vibration.SmallMotor)
	})

	if err != nil {
		return fmt.Errorf("unable to start ViGEm client: %w", err)
	}

	defer emulator.Close()

	x360, err := emulator.CreateXbox360Controller()

	if err != nil {
		return fmt.Errorf("unable to create emulated Xbox 360 controller: %w", err)
	}

	defer x360.Close()

	if err = x360.Connect(); err != nil {
		return fmt.Errorf("unable to connect to emulated Xbox 360 controller: %w", err)
	}

	assistantPressed, capturePressed := false, false

	for {
		report, err := controller.GetReport()

		if err != nil {
			if errors.Is(err, stadiacontroller.RetryError) {
				time.Sleep(1 * time.Second)
				continue
			}
			return err
		}

		err = x360.Send(&report)

		if err != nil {
			return err
		}

		if report.Assistant != assistantPressed {
			assistantPressed = report.Assistant

			if err := runButtonPress(assistantPressed, *onAssistantPressed, *onAssistantReleased); err != nil {
				return err
			}
		}

		if report.Capture != capturePressed {
			capturePressed = report.Capture

			if err := runButtonPress(capturePressed, *onCapturePressed, *onCaptureReleased); err != nil {
				return err
			}
		}
	}
}

func runButtonPress(pressed bool, ifPressed, ifReleased string) error {
	if pressed && ifPressed != "" {
		return runCommand(ifPressed)
	}
	if !pressed && ifReleased != "" {
		return runCommand(ifReleased)
	}
	return nil
}

func runCommand(cmd string) error {
	command := exec.Command(*shell, "/C", cmd)

	if err := command.Start(); err != nil {
		return err
	}

	go func() {
		err := command.Wait()

		if err != nil {
			log.Printf("command '%s' failed: %v", cmd, err)
		}
	}()

	return nil
}
