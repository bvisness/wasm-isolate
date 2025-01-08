package isolate

import (
	"errors"
	"fmt"
	"io"

	"github.com/bvisness/wasm-isolate/leb128"
)

func Isolate(wasm io.Reader, out io.Writer, funcs []int) error {
	p := newParser(wasm)

	if err := p.Expect("magic number", []byte{0, 'a', 's', 'm'}); err != nil {
		return err
	}
	if err := p.Expect("version number", []byte{1, 0, 0, 0}); err != nil {
		return err
	}

	for {
		sectionId, err := p.ReadByte("section id")
		if errors.Is(err, io.EOF) {
			break
		} else if err != nil {
			return err
		}
		sectionSize, err := p.ReadU32("section size")
		if err != nil {
			return err
		}

		switch sectionId {
		case 3: // function section
			numFuncs, err := p.ReadU32("num funcs")
			if err != nil {
				return err
			}
			for i := range numFuncs {
				// TODO: I dunno
			}
		// case 9: // element section
		// TODO: Parse segments of funcref to make sure we don't strip declared funcs
		// case 10: // code section
		default:
			fmt.Fprintf(out, "section with ID %d and size %d\n", sectionId, sectionSize)
			if _, err := p.ReadN("section contents", int(sectionSize)); err != nil {
				return err
			}
		}
	}

	out.Write([]byte("wow!\n"))

	return nil
}

type parser struct {
	r   io.Reader
	cur int
}

func newParser(r io.Reader) parser {
	return parser{
		r:   r,
		cur: 0,
	}
}

func (p *parser) ReadN(thing string, n int) ([]byte, error) {
	at := p.cur
	bytes := make([]byte, n)
	nRead, err := io.ReadFull(p.r, bytes)
	if err != nil {
		return nil, fmt.Errorf("%s at offset %d: %w", thing, at, err)
	}
	p.cur += nRead
	return bytes, nil
}

func (p *parser) ReadByte(thing string) (byte, error) {
	at := p.cur
	var b [1]byte
	_, err := io.ReadFull(p.r, b[:])
	if err != nil {
		return 0, fmt.Errorf("%s at offset %d: %w", thing, at, err)
	}
	p.cur += 1
	return b[0], nil
}

func (p *parser) ReadU32(thing string) (uint32, error) {
	at := p.cur
	v, n, err := leb128.DecodeU64(p.r)
	if err != nil {
		return 0, fmt.Errorf("%s at offset %d: %w", thing, at, err)
	}
	p.cur += n
	return uint32(v), nil
}

func (p *parser) Expect(thing string, bytes []byte) error {
	at := p.cur
	actual, err := p.ReadN(thing, len(bytes))
	if err != nil {
		return err
	}
	if err := p.AssertBytesEqual(at, actual, bytes); err != nil {
		return fmt.Errorf("reading %s: %w", thing, err)
	}
	return nil
}

func (p *parser) AssertBytesEqual(at int, actual, expected []byte) error {
	if len(actual) != len(expected) {
		return fmt.Errorf("at offset %d: expected bytes %+v but got %+v", at, expected, actual)
	}
	for i := range actual {
		if actual[i] != expected[i] {
			return fmt.Errorf("at offset %d: expected bytes %+v but got %+v", at, expected, actual)
		}
	}
	return nil
}
