package easynet

import (
	"context"
	"errors"
)

type carrierState uint8

const (
	carrierOpen carrierState = iota
	carrierClosing
	carrierClosed
	carrierFailed
)

func (s carrierState) open() bool {
	return s == carrierOpen
}

func isLocalCarrierInterruption(err error) bool {
	return errors.Is(err, context.Canceled) ||
		errors.Is(err, context.DeadlineExceeded) ||
		IsCode(err, ErrCancelled) ||
		IsCode(err, ErrTimeout)
}
