// Package leb128 provides functionality to encode/decode signed and unsigned LEB128 data to and from 8 byte primitives.
//
// It deals only with 8 byte primitives; attempting to decode integers larger than that will cause an ErrOverflow.
//
// This package operates on a basic io.Reader rather than an io.ByteReader as the standard library does (i.e. the various Varint functions in https://pkg.go.dev/encoding/binary).
//
// See https://en.wikipedia.org/wiki/LEB128 for more details.
package leb128

import (
	"errors"
	"io"
)

const (
	// maxWidth indicates the maximum number of bytes that can be used
	// to encode an 8 byte integer in LEB128
	maxWidth = 10
)

var (
	ErrOverflow = errors.New("LEB128 integer overflow (was more than 8 bytes)")
)

// DecodeU64 converts a uleb128 byte stream to a uint64. Be careful
// to ensure that your data can fit in 8 bytes.
func DecodeU64(r io.Reader) (uint64, int, error) {
	var res uint64

	numBytes := 0
	bit := int8(0)
	buf := make([]byte, 1)
	for i := 0; ; i++ {
		if i >= maxWidth {
			return 0, numBytes, ErrOverflow
		}

		n, err := r.Read(buf)
		numBytes += n
		if err == io.EOF {
			break
		}
		if err != nil {
			return 0, numBytes, err
		}
		b := buf[0]

		res |= uint64(b&0x7f) << (7 * bit)

		signBit := b & 0x80
		if signBit == 0 {
			break
		}

		bit++
	}

	return res, numBytes, nil
}

// DecodeS64 converts a sleb128 byte stream to a int64. Be careful
// to ensure that your data can fit in 8 bytes.
func DecodeS64(r io.Reader) (int64, int, error) {
	var res int64

	numBytes := 0
	shift := 0
	buf := make([]byte, 1)
	for i := 0; ; i++ {
		if i >= maxWidth {
			return 0, numBytes, ErrOverflow
		}

		n, err := r.Read(buf)
		numBytes += n
		if err == io.EOF {
			break
		}
		if err != nil {
			return 0, numBytes, err
		}
		b := buf[0]

		res |= int64(b&0x7f) << shift
		shift += 7

		if b&0x80 == 0 {
			if b&0x40 != 0 {
				// signed
				res |= ^0 << shift
			}
			break
		}
	}

	return res, numBytes, nil
}

// EncodeU64 converts num to a uleb128 encoded array of bytes
func EncodeU64(num uint64) []byte {
	buf := make([]byte, 0)

	done := false
	for !done {
		b := byte(num & 0x7F)

		num = num >> 7
		if num == 0 {
			done = true
		} else {
			b |= 0x80
		}

		buf = append(buf, b)
	}

	return buf
}

// EncodeS64 converts num to a sleb128 encoded array of bytes
func EncodeS64(num int64) []byte {
	buf := make([]byte, 0)

	done := false
	for !done {
		//
		// From https://go.dev/ref/spec#Arithmetic_operators:
		//
		// "The shift operators implement arithmetic shifts
		// if the left operand is a signed integer and
		// logical shifts if it is an unsigned integer"
		//

		b := byte(num & 0x7F)
		num >>= 7 // arithmetic shift
		signBit := b & 0x40
		if (num == 0 && signBit == 0) ||
			(num == -1 && signBit != 0) {
			done = true
		} else {
			b |= 0x80
		}

		buf = append(buf, b)
	}

	return buf
}
