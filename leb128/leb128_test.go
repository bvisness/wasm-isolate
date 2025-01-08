package leb128_test

import (
	"bytes"
	"fmt"
	"math"
	"testing"

	"github.com/bvisness/wasm-isolate/leb128"
	"github.com/stretchr/testify/require"
)

type errorReader struct{}

func (er *errorReader) Read(_ []byte) (int, error) {
	return 0, fmt.Errorf("test error")
}

func TestUnsigned(t *testing.T) {
	t.Run("simple low-range cases", func(t *testing.T) {
		for ndx := uint64(0); ndx < 512; ndx++ {
			buf := leb128.EncodeU64(ndx)
			var expectedLen int
			require.NotEmpty(t, buf)
			if ndx >= 384 { // [384,512)
				// i.e. 384 => [128,3]
				expectedLen = 2
				require.Len(t, buf, expectedLen)
				require.Equal(t, byte(ndx), buf[0])
				require.Equal(t, byte(3), buf[1])
			} else if ndx >= 256 { // [256,384)
				// i.e. 256 => [128,2]
				expectedLen = 2
				require.Len(t, buf, expectedLen)
				require.Equal(t, byte(ndx-128), buf[0])
				require.Equal(t, byte(2), buf[1])
			} else if ndx >= 128 { // [128,256)
				// i.e. 256 => [128,1]
				expectedLen = 2
				require.Len(t, buf, expectedLen)
				require.Equal(t, byte(ndx), buf[0])
				require.Equal(t, byte(1), buf[1])
			} else { // [0,128)
				expectedLen = 1
				require.Len(t, buf, expectedLen)
				require.Equal(t, byte(ndx), buf[0])
			}

			// translate back
			res, n, err := leb128.DecodeU64(bytes.NewBuffer(buf))
			require.NoError(t, err)
			require.Equal(t, expectedLen, n)
			require.Equal(t, ndx, res)
		}
	})

	t.Run("max uint64", func(t *testing.T) {
		expected := []byte{0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01}

		buf := leb128.EncodeU64(math.MaxUint64)
		require.Equal(t, expected, buf)

		// translate back
		res, n, err := leb128.DecodeU64(bytes.NewBuffer(buf))
		require.NoError(t, err)
		require.Equal(t, 10, n)
		require.Equal(t, uint64(math.MaxUint64), res)
	})

	t.Run("empty buffer", func(t *testing.T) {
		res, n, err := leb128.DecodeU64(bytes.NewBuffer([]byte{}))
		require.NoError(t, err)
		require.Equal(t, 0, n)
		require.Zero(t, res)
	})

	t.Run("read error", func(t *testing.T) {
		res, n, err := leb128.DecodeU64(&errorReader{})
		require.Error(t, err)
		require.Equal(t, 0, n)
		require.Zero(t, res)
	})

	t.Run("ensure that we stop at the correct time", func(t *testing.T) {
		input := []byte{0x78, 0x10, 0xf, 0xa, 0xb, 0x90, 0x01, 0, 0xff, 0xff, 0xff}
		res, n, err := leb128.DecodeU64(bytes.NewBuffer(input))
		require.NoError(t, err)
		require.Equal(t, 1, n)
		require.Equal(t, uint64(120), res)
	})

	t.Run("restrict to 10 bytes (final bytes would overflow an 8 byte integer)", func(t *testing.T) {
		input := []byte{0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x0}

		res, n, err := leb128.DecodeU64(bytes.NewBuffer(input))
		require.ErrorIs(t, err, leb128.ErrOverflow)
		require.Equal(t, 10, n)
		require.Equal(t, uint64(0), res)
	})
}

func TestSigned(t *testing.T) {
	t.Run("simple low-range positive cases", func(t *testing.T) {
		for ndx := int64(0); ndx < 512; ndx++ {
			buf := leb128.EncodeS64(ndx)
			require.NotEmpty(t, buf)
			var expectedLen int
			if ndx >= 384 { // [384,512)
				// i.e. 384 => [128,3]
				expectedLen = 2
				require.Len(t, buf, expectedLen)
				require.Equal(t, byte(ndx), buf[0])
				require.Equal(t, byte(3), buf[1])
			} else if ndx >= 256 { // [256,384)
				// i.e. 256 => [128,2]
				expectedLen = 2
				require.Len(t, buf, expectedLen)
				require.Equal(t, byte(ndx+128), buf[0])
				require.Equal(t, byte(2), buf[1])
			} else if ndx >= 128 { // [128,256)
				// i.e. 256 => [128,1]
				expectedLen = 2
				require.Len(t, buf, expectedLen)
				require.Equal(t, byte(ndx), buf[0])
				require.Equal(t, byte(1), buf[1])
			} else if ndx >= 64 { // [0,64)
				// i.e. 64 => [192,1]
				expectedLen = 2
				require.Len(t, buf, expectedLen)
				require.Equal(t, byte(ndx+128), buf[0])
				require.Equal(t, byte(0), buf[1])
			} else { // [0,64)
				expectedLen = 1
				require.Len(t, buf, expectedLen)
				require.Equal(t, byte(ndx), buf[0])
			}

			// translate back
			res, n, err := leb128.DecodeS64(bytes.NewBuffer(buf))
			require.NoError(t, err)
			require.Equal(t, expectedLen, n)
			require.Equal(t, ndx, res)
		}
	})

	t.Run("simple low-range negative cases", func(t *testing.T) {
		for ndx := int64(-512); ndx < 0; ndx++ {
			buf := leb128.EncodeS64(ndx)
			require.NotEmpty(t, buf)
			var expectedLen int
			if ndx < -384 { // [-512,-384)
				// i.e. -512 => [128,124]
				expectedLen = 2
				require.Len(t, buf, expectedLen)
				require.Equal(t, byte(ndx+128), buf[0])
				require.Equal(t, byte(124), buf[1])
			} else if ndx < -256 { // [-384,-256)
				// i.e. -384 => [128,125]
				expectedLen = 2
				require.Len(t, buf, expectedLen)
				require.Equal(t, byte(ndx), buf[0])
				require.Equal(t, byte(125), buf[1])
			} else if ndx < -128 { // [-256,-128)
				// i.e. -256 => [128,126]
				expectedLen = 2
				require.Len(t, buf, expectedLen)
				require.Equal(t, byte(ndx+128), buf[0])
				require.Equal(t, byte(126), buf[1])
			} else if ndx < -64 { // [-128,-64)
				// i.e. -128 => [128,127]
				expectedLen = 2
				require.Len(t, buf, expectedLen)
				require.Equal(t, byte(ndx), buf[0])
				require.Equal(t, byte(127), buf[1])
			} else {
				expectedLen = 1
				require.Len(t, buf, expectedLen)
				require.Equal(t, byte(ndx+128), buf[0])
			}

			// translate back
			res, n, err := leb128.DecodeS64(bytes.NewBuffer(buf))
			require.NoError(t, err)
			require.Equal(t, expectedLen, n)
			require.Equal(t, ndx, res)
		}
	})

	t.Run("max int64", func(t *testing.T) {
		expected := []byte{0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0}

		buf := leb128.EncodeS64(math.MaxInt64)
		require.Equal(t, expected, buf)

		// translate back
		res, n, err := leb128.DecodeS64(bytes.NewBuffer(buf))
		require.NoError(t, err)
		require.Equal(t, 10, n)
		require.Equal(t, int64(math.MaxInt64), res)
	})

	t.Run("min int64", func(t *testing.T) {
		expected := []byte{0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x7f}

		buf := leb128.EncodeS64(math.MinInt64)
		require.Equal(t, expected, buf)

		// translate back
		res, n, err := leb128.DecodeS64(bytes.NewBuffer(buf))
		require.NoError(t, err)
		require.Equal(t, 10, n)
		require.Equal(t, int64(math.MinInt64), res)
	})

	t.Run("empty buffer", func(t *testing.T) {
		res, n, err := leb128.DecodeS64(bytes.NewBuffer([]byte{}))
		require.NoError(t, err)
		require.Equal(t, 0, n)
		require.Zero(t, res)
	})

	t.Run("read error", func(t *testing.T) {
		res, n, err := leb128.DecodeS64(&errorReader{})
		require.Error(t, err)
		require.Equal(t, 0, n)
		require.Zero(t, res)
	})

	t.Run("restrict to 10 bytes (final bytes overflow an 8 byte integer)", func(t *testing.T) {
		input := []byte{0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0xff}

		res, n, err := leb128.DecodeS64(bytes.NewBuffer(input))
		require.ErrorIs(t, err, leb128.ErrOverflow)
		require.Equal(t, 10, n)
		require.Equal(t, int64(0), res)
	})
}
