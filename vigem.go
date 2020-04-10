package stadiacontroller

/*
#include <stdint.h>

typedef struct
{
	uint16_t wButtons;
	uint8_t bLeftTrigger;
	uint8_t bRightTrigger;
	int16_t sThumbLX;
	int16_t sThumbLY;
	int16_t sThumbRX;
	int16_t sThumbRY;
} xusb_report;
*/
import "C"

import (
	"errors"
	"unsafe"

	"golang.org/x/sys/windows"
)

const (
	VIGEM_ERROR_NONE                        = 0x20000000
	VIGEM_ERROR_BUS_NOT_FOUND               = 0xE0000001
	VIGEM_ERROR_NO_FREE_SLOT                = 0xE0000002
	VIGEM_ERROR_INVALID_TARGET              = 0xE0000003
	VIGEM_ERROR_REMOVAL_FAILED              = 0xE0000004
	VIGEM_ERROR_ALREADY_CONNECTED           = 0xE0000005
	VIGEM_ERROR_TARGET_UNINITIALIZED        = 0xE0000006
	VIGEM_ERROR_TARGET_NOT_PLUGGED_IN       = 0xE0000007
	VIGEM_ERROR_BUS_VERSION_MISMATCH        = 0xE0000008
	VIGEM_ERROR_BUS_ACCESS_FAILED           = 0xE0000009
	VIGEM_ERROR_CALLBACK_ALREADY_REGISTERED = 0xE0000010
	VIGEM_ERROR_CALLBACK_NOT_FOUND          = 0xE0000011
	VIGEM_ERROR_BUS_ALREADY_CONNECTED       = 0xE0000012
	VIGEM_ERROR_BUS_INVALID_HANDLE          = 0xE0000013
	VIGEM_ERROR_XUSB_USERINDEX_OUT_OF_RANGE = 0xE0000014

	VIGEM_ERROR_MAX = VIGEM_ERROR_XUSB_USERINDEX_OUT_OF_RANGE + 1
)

var (
	client = windows.NewLazyDLL("ViGEmClient.dll")

	procAlloc                            = client.NewProc("vigem_alloc")
	procFree                             = client.NewProc("vigem_free")
	procConnect                          = client.NewProc("vigem_connect")
	procDisconnect                       = client.NewProc("vigem_disconnect")
	procTargetAdd                        = client.NewProc("vigem_target_add")
	procTargetFree                       = client.NewProc("vigem_target_free")
	procTargetRemove                     = client.NewProc("vigem_target_remove")
	procTargetX360Alloc                  = client.NewProc("vigem_target_x360_alloc")
	procTargetX360RegisterNotification   = client.NewProc("vigem_target_x360_register_notification")
	procTargetX360UnregisterNotification = client.NewProc("vigem_target_x360_unregister_notification")
	procTargetX360Update                 = client.NewProc("vigem_target_x360_update")
)

type VigemError struct {
	code uint
}

func NewVigemError(rawCode uintptr) *VigemError {
	code := uint(rawCode)

	if code == VIGEM_ERROR_NONE {
		return nil
	}

	return &VigemError{code}
}

func (err *VigemError) Error() string {
	switch err.code {
	case VIGEM_ERROR_BUS_NOT_FOUND:
		return "bus not found"
	case VIGEM_ERROR_NO_FREE_SLOT:
		return "no free slot"
	case VIGEM_ERROR_INVALID_TARGET:
		return "invalid target"
	case VIGEM_ERROR_REMOVAL_FAILED:
		return "removal failed"
	case VIGEM_ERROR_ALREADY_CONNECTED:
		return "already connected"
	case VIGEM_ERROR_TARGET_UNINITIALIZED:
		return "target uninitialized"
	case VIGEM_ERROR_TARGET_NOT_PLUGGED_IN:
		return "target not plugged in"
	case VIGEM_ERROR_BUS_VERSION_MISMATCH:
		return "bus version mismatch"
	case VIGEM_ERROR_BUS_ACCESS_FAILED:
		return "bus access failed"
	case VIGEM_ERROR_CALLBACK_ALREADY_REGISTERED:
		return "callback already registered"
	case VIGEM_ERROR_CALLBACK_NOT_FOUND:
		return "callback not found"
	case VIGEM_ERROR_BUS_ALREADY_CONNECTED:
		return "bus already connected"
	case VIGEM_ERROR_BUS_INVALID_HANDLE:
		return "bus invalid handle"
	case VIGEM_ERROR_XUSB_USERINDEX_OUT_OF_RANGE:
		return "xusb userindex out of range"
	default:
		return "invalid code returned by ViGEm"
	}
}

type Emulator struct {
	handle      uintptr
	onVibration func(vibration Vibration)
}

type Vibration struct {
	LargeMotor byte
	SmallMotor byte
}

func NewEmulator(onVibration func(vibration Vibration)) (*Emulator, error) {
	handle, _, err := procAlloc.Call()

	if !errors.Is(err, windows.ERROR_SUCCESS) {
		return nil, err
	}

	libErr, _, err := procConnect.Call(handle)

	if !errors.Is(err, windows.ERROR_SUCCESS) {
		return nil, err
	}
	if err := NewVigemError(libErr); err != nil {
		return nil, err
	}

	return &Emulator{handle, onVibration}, nil
}

func (e *Emulator) Close() error {
	procDisconnect.Call(e.handle)
	_, _, err := procFree.Call(e.handle)

	return err
}

func (e *Emulator) CreateXbox360Controller() (*Xbox360Controller, error) {
	handle, _, err := procTargetX360Alloc.Call()

	if !errors.Is(err, windows.ERROR_SUCCESS) {
		return nil, err
	}

	notificationHandler := func(client, target uintptr, largeMotor, smallMotor, ledNumber byte) uintptr {
		e.onVibration(Vibration{largeMotor, smallMotor})

		return 0
	}
	callback := windows.NewCallback(notificationHandler)

	return &Xbox360Controller{e, handle, false, callback}, nil
}

type x360NotificationHandler func(client, target uintptr, largeMotor, smallMotor, ledNumber byte) uintptr

type Xbox360Controller struct {
	emulator            *Emulator
	handle              uintptr
	connected           bool
	notificationHandler uintptr
}

func (c *Xbox360Controller) Close() error {
	_, _, err := procTargetFree.Call(c.handle)

	return err
}

func (c *Xbox360Controller) Connect() error {
	libErr, _, err := procTargetAdd.Call(c.emulator.handle, c.handle)

	if !errors.Is(err, windows.ERROR_SUCCESS) {
		return err
	}
	if err := NewVigemError(libErr); err != nil {
		return err
	}

	libErr, _, err = procTargetX360RegisterNotification.Call(c.emulator.handle, c.handle, c.notificationHandler)

	if !errors.Is(err, windows.ERROR_SUCCESS) {
		return err
	}
	if err := NewVigemError(libErr); err != nil {
		return err
	}

	c.connected = true

	return nil
}

func (c *Xbox360Controller) Disconnect() error {
	libErr, _, err := procTargetX360UnregisterNotification.Call(c.handle)

	if !errors.Is(err, windows.ERROR_SUCCESS) {
		return err
	}
	if err := NewVigemError(libErr); err != nil {
		return err
	}

	libErr, _, err = procTargetRemove.Call(c.emulator.handle, c.handle)

	if !errors.Is(err, windows.ERROR_SUCCESS) {
		return err
	}
	if err := NewVigemError(libErr); err != nil {
		return err
	}

	c.connected = false

	return nil
}

func (c *Xbox360Controller) Send(report *Xbox360ControllerReport) error {
	libErr, _, err := procTargetX360Update.Call(c.emulator.handle, c.handle, uintptr(unsafe.Pointer(&report.native)))

	if !errors.Is(err, windows.ERROR_SUCCESS) {
		return err
	}
	if err := NewVigemError(libErr); err != nil {
		return err
	}

	return nil
}

type Xbox360ControllerReport struct {
	native    C.xusb_report
	Capture   bool
	Assistant bool
}

// Bits that correspond to the Xbox 360 controller buttons.
const (
	Xbox360ControllerButtonUp            = 0
	Xbox360ControllerButtonDown          = 1
	Xbox360ControllerButtonLeft          = 2
	Xbox360ControllerButtonRight         = 3
	Xbox360ControllerButtonStart         = 4
	Xbox360ControllerButtonBack          = 5
	Xbox360ControllerButtonLeftThumb     = 6
	Xbox360ControllerButtonRightThumb    = 7
	Xbox360ControllerButtonLeftShoulder  = 8
	Xbox360ControllerButtonRightShoulder = 9
	Xbox360ControllerButtonGuide         = 10
	Xbox360ControllerButtonA             = 12
	Xbox360ControllerButtonB             = 13
	Xbox360ControllerButtonX             = 14
	Xbox360ControllerButtonY             = 15
)

func NewXbox360ControllerReport() Xbox360ControllerReport {
	return Xbox360ControllerReport{}
}

func (r *Xbox360ControllerReport) GetButtons() uint16 {
	return uint16(r.native.wButtons)
}

func (r *Xbox360ControllerReport) SetButtons(buttons uint16) {
	r.native.wButtons = C.uint16_t(buttons)
}

func (r *Xbox360ControllerReport) MaybeSetButton(shiftBy int, isSet bool) {
	if isSet {
		r.SetButton(shiftBy)
	}
}

func (r *Xbox360ControllerReport) SetButton(shiftBy int) {
	r.native.wButtons |= 1 << shiftBy
}

func (r *Xbox360ControllerReport) GetLeftTrigger() byte {
	return byte(r.native.bLeftTrigger)
}

func (r *Xbox360ControllerReport) SetLeftTrigger(value byte) {
	r.native.bLeftTrigger = C.uint8_t(value)
}

func (r *Xbox360ControllerReport) GetRightTrigger() byte {
	return byte(r.native.bRightTrigger)
}

func (r *Xbox360ControllerReport) SetRightTrigger(value byte) {
	r.native.bRightTrigger = C.uint8_t(value)
}

func (r *Xbox360ControllerReport) GetLeftThumb() (x, y int16) {
	return int16(r.native.sThumbLX), int16(r.native.sThumbLY)
}

func (r *Xbox360ControllerReport) SetLeftThumb(x, y int16) {
	r.native.sThumbLX = C.int16_t(x)
	r.native.sThumbLY = C.int16_t(y)
}

func (r *Xbox360ControllerReport) GetRightThumb() (x, y int16) {
	return int16(r.native.sThumbRX), int16(r.native.sThumbRY)
}

func (r *Xbox360ControllerReport) SetRightThumb(x, y int16) {
	r.native.sThumbRX = C.int16_t(x)
	r.native.sThumbRY = C.int16_t(y)
}
