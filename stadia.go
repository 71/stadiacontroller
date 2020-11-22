package stadiacontroller

import (
	"encoding/base64"
	"errors"
	"fmt"
	"log"
	"time"
)

const (
	stadiaControllerVid = 0x18D1
	stadiaControllerPid = 0x9400
)

type StadiaController struct {
	device *Device
	ticker *time.Ticker
	err    error
}

func NewStadiaController() *StadiaController {
	ticker := time.NewTicker(1 * time.Second)
	controller := &StadiaController{nil, ticker, nil}

	go func() {
		for range ticker.C {
			if controller.device != nil || controller.err != nil {
				continue
			}

			devices, err := Devices()

			if err != nil {
				controller.err = err

				break
			}

			for _, device := range devices {
				if device.VendorID == stadiaControllerVid && device.ProductID == stadiaControllerPid {
					openDevice, err := device.Open()

					if err != nil {
						log.Printf("cannot open device %s: %v", device.Path, err)

						break
					}

					log.Printf("opened device %s", device.Path)
					controller.device = &openDevice

					break
				}
			}
		}
	}()

	return controller
}

func (c *StadiaController) Close() {
	c.ticker.Stop()

	if c.device == nil {
		return
	}

	(*c.device).Close()
}

func (c *StadiaController) Vibrate(largeMotor, smallMotor byte) error {
	if c.device == nil {
		return c.err
	}

	return (*c.device).Write([]byte{0x05, largeMotor, largeMotor, smallMotor, smallMotor})
}

var RetryError = errors.New("retry")

func (c *StadiaController) GetReport() (Xbox360ControllerReport, error) {
	report := Xbox360ControllerReport{}

	if c.device == nil {
		err := c.err
		if err == nil {
			err = RetryError
		}
		return report, err
	}

	buf, ok := <-(*c.device).ReadCh()

	if !ok {
		err := (*c.device).ReadError()
		log.Printf("unable to read from controller: %v", err)
		log.Printf("waiting for new controller")
		(*c.device).Close()
		c.device = nil
		return report, RetryError
	}

	err := ParseReport(buf, &report)

	if err != nil {
		log.Printf("unable to parse controller report: %v", err)
		return report, RetryError
	}

	return report, nil
}

func ParseReport(data []byte, report *Xbox360ControllerReport) error {
	if len(data) == 0 {
		return errors.New("cannot parse empty report")
	}

	if data[0] == 0x03 && len(data) >= 10 {
		a := data[1]
		b := data[2]
		c := data[3]

		// Update common buttons.
		report.MaybeSetButton(Xbox360ControllerButtonA, (c&0b0100_0000) != 0)
		report.MaybeSetButton(Xbox360ControllerButtonB, (c&0b0010_0000) != 0)
		report.MaybeSetButton(Xbox360ControllerButtonX, (c&0b0001_0000) != 0)
		report.MaybeSetButton(Xbox360ControllerButtonY, (c&0b0000_1000) != 0)
		report.MaybeSetButton(Xbox360ControllerButtonLeftShoulder, (c&0b0000_0100) != 0)
		report.MaybeSetButton(Xbox360ControllerButtonRightShoulder, (c&0b0000_0010) != 0)
		report.MaybeSetButton(Xbox360ControllerButtonLeftThumb, (c&0b0000_0001) != 0)
		report.MaybeSetButton(Xbox360ControllerButtonRightThumb, (b&0b1000_0000) != 0)
		report.MaybeSetButton(Xbox360ControllerButtonBack, (b&0b0100_0000) != 0)
		report.MaybeSetButton(Xbox360ControllerButtonStart, (b&0b0010_0000) != 0)
		report.MaybeSetButton(Xbox360ControllerButtonGuide, (b&0b0001_0000) != 0)

		report.Assistant = (b & 0b0000_0010) != 0
		report.Capture = (b & 0b0000_0001) != 0

		// Update DPad buttons.
		switch a {
		case 0:
			report.SetButton(Xbox360ControllerButtonUp)
		case 1:
			report.SetButton(Xbox360ControllerButtonUp)
			report.SetButton(Xbox360ControllerButtonRight)
		case 2:
			report.SetButton(Xbox360ControllerButtonRight)
		case 3:
			report.SetButton(Xbox360ControllerButtonRight)
			report.SetButton(Xbox360ControllerButtonDown)
		case 4:
			report.SetButton(Xbox360ControllerButtonDown)
		case 5:
			report.SetButton(Xbox360ControllerButtonDown)
			report.SetButton(Xbox360ControllerButtonLeft)
		case 6:
			report.SetButton(Xbox360ControllerButtonLeft)
		case 7:
			report.SetButton(Xbox360ControllerButtonLeft)
			report.SetButton(Xbox360ControllerButtonUp)
		}

		// Normalize axes values.
		// Port of https://github.com/MWisBest/StadiEm.
		for i := 4; i < 8; i++ {
			if data[i] <= 0x7F && data[i] > 0x00 {
				data[i]--
			}
		}

		// Set axes values.
		lThumbX := convertAxisValue(data[4]) - 0x8000
		lThumbY := -convertAxisValue(data[5]) + 0x7fff
		rThumbX := convertAxisValue(data[6]) - 0x8000
		rThumbY := -convertAxisValue(data[7]) + 0x7fff

		if lThumbY == -1 {
			lThumbY = 0
		}
		if rThumbY == -1 {
			rThumbY = 0
		}

		report.SetLeftThumb(int16(lThumbX), int16(lThumbY))
		report.SetRightThumb(int16(rThumbX), int16(rThumbY))

		// Set triggers.
		report.SetLeftTrigger(data[8])
		report.SetRightTrigger(data[9])

		return nil
	}

	return fmt.Errorf("unknown report format; raw report was %s", base64.StdEncoding.EncodeToString(data))
}

func convertAxisValue(byteValue byte) int32 {
	value := int32(byteValue)
	value = value<<8 | ((value << 1) & 0b1111)

	if value == 0xfffe {
		return 0xffff
	}

	return value
}
