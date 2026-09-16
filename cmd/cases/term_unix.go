//go:build darwin || linux

package main

import (
	"os"
	"syscall"
	"unsafe"
)

// keysRaw turns off line buffering and echo on the terminal f, so serve sees
// each key as it is pressed. Signals stay on: Ctrl-C still sends SIGINT. The
// returned function puts the terminal back as it was.
func keysRaw(f *os.File) (restore func(), err error) {
	fd := f.Fd()
	var old syscall.Termios
	if err := termios(fd, ioctlGetTermios, &old); err != nil {
		return nil, err
	}
	raw := old
	raw.Lflag &^= syscall.ICANON | syscall.ECHO
	raw.Cc[syscall.VMIN] = 1
	raw.Cc[syscall.VTIME] = 0
	if err := termios(fd, ioctlSetTermios, &raw); err != nil {
		return nil, err
	}
	return func() { _ = termios(fd, ioctlSetTermios, &old) }, nil
}

// hasTermios reports whether f answers a terminal query.
func hasTermios(f *os.File) bool {
	var t syscall.Termios
	return termios(f.Fd(), ioctlGetTermios, &t) == nil
}

func termios(fd uintptr, req uintptr, t *syscall.Termios) error {
	if _, _, errno := syscall.Syscall(syscall.SYS_IOCTL, fd, req, uintptr(unsafe.Pointer(t))); errno != 0 {
		return errno
	}
	return nil
}

// isForeground reports whether this process is in the foreground process
// group of the terminal f. A background job that reads the terminal or
// changes its modes is stopped by SIGTTIN or SIGTTOU.
func isForeground(f *os.File) bool {
	var pgrp int32
	if _, _, errno := syscall.Syscall(syscall.SYS_IOCTL, f.Fd(), syscall.TIOCGPGRP, uintptr(unsafe.Pointer(&pgrp))); errno != 0 {
		return false
	}
	return int(pgrp) == syscall.Getpgrp()
}

// termRows returns the height of the terminal f, or 0 when it cannot tell.
func termRows(f *os.File) int {
	var ws struct{ Row, Col, Xpixel, Ypixel uint16 }
	if _, _, errno := syscall.Syscall(syscall.SYS_IOCTL, f.Fd(), syscall.TIOCGWINSZ, uintptr(unsafe.Pointer(&ws))); errno != 0 {
		return 0
	}
	return int(ws.Row)
}
