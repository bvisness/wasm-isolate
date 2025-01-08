package isolate

import (
	"bufio"
	"errors"
	"fmt"
	"io"

	"github.com/bvisness/wasm-isolate/leb128"
	"github.com/bvisness/wasm-isolate/utils"
)

func Isolate(wasm io.Reader, out io.Writer, funcs []int) error {
	p := newParser(wasm)

	if err := p.Expect("magic number", []byte{0, 'a', 's', 'm'}); err != nil {
		return err
	}
	if err := p.Expect("version number", []byte{1, 0, 0, 0}); err != nil {
		return err
	}

	numFuncs := 0

	for {
		sectionId, err := p.ReadByte("section id")
		if errors.Is(err, io.EOF) {
			break
		} else if err != nil {
			return err
		}
		sectionSize, _, err := p.ReadU32("section size")
		if err != nil {
			return err
		}

		switch sectionId {
		case 2: // import section
			numImports, _, err := p.ReadU32("num imports")
			if err != nil {
				return err
			}
			for range numImports {
				_, err = p.ReadName("import module")
				if err != nil {
					return err
				}
				_, err = p.ReadName("import name")
				if err != nil {
					return err
				}

				importType, err := p.ReadByte("import type")
				if err != nil {
					return err
				}
				switch importType {
				case 0x00: // function
					numFuncs++
					_, _, err := p.ReadU32("type of imported function")
					if err != nil {
						return err
					}
				case 0x01: // table
					_, err := p.ReadTableType("type of imported table")
					if err != nil {
						return err
					}
				case 0x02: // memory
					_, err := p.ReadMemType("type of imported memory")
					if err != nil {
						return err
					}
				case 0x03: // global
					_, err := p.ReadGlobalType("type of imported global")
					if err != nil {
						return err
					}
				case 0x04: // tag
					_, err := p.ReadTagType("type of imported tag")
					if err != nil {
						return err
					}
				}
			}
		// case 3: // function section
		// 	numFuncs, _, err := p.ReadU32("num funcs")
		// 	if err != nil {
		// 		return err
		// 	}
		// 	for range numFuncs {
		// 		// TODO: copy functions that we want to preserve, and track their relocations
		// 	}
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

	fmt.Fprintf(out, "wow! %d imported functions\n", numFuncs)

	return nil
}

type parser struct {
	r   *bufio.Reader
	cur int
}

func newParser(r io.Reader) parser {
	return parser{
		r:   bufio.NewReader(r),
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

func (p *parser) PeekByte(thing string) (byte, error) {
	at := p.cur
	bytes, err := p.r.Peek(1)
	if err != nil {
		return 0, fmt.Errorf("%s at offset %d: %w", thing, at, err)
	}
	return bytes[0], nil
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

func (p *parser) ReadU32(thing string) (uint32, int, error) {
	v, n, err := p.ReadU64(thing)
	return uint32(v), n, err
}

func (p *parser) ReadU64(thing string) (uint64, int, error) {
	at := p.cur
	v, n, err := leb128.DecodeU64(p.r)
	if err != nil {
		return 0, n, fmt.Errorf("%s at offset %d: %w", thing, at, err)
	}
	p.cur += n
	return v, n, nil
}

func (p *parser) ReadS32(thing string) (int32, int, error) {
	v, n, err := p.ReadS64(thing)
	return int32(v), n, err
}

func (p *parser) ReadS64(thing string) (int64, int, error) {
	at := p.cur
	v, n, err := leb128.DecodeS64(p.r)
	if err != nil {
		return 0, n, fmt.Errorf("%s at offset %d: %w", thing, at, err)
	}
	p.cur += n
	return v, n, nil
}

func (p *parser) ReadName(thing string) (string, error) {
	n, _, err := p.ReadU32(thing)
	if err != nil {
		return "", err
	}
	name, err := p.ReadN(thing, int(n))
	if err != nil {
		return "", err
	}
	return string(name), nil
}

func (p *parser) ReadTableType(thing string) (tableType, error) {
	et, err := p.ReadRefType(fmt.Sprintf("element type for %s", thing))
	if err != nil {
		return tableType{}, err
	}
	lim, err := p.ReadLimits(fmt.Sprintf("limits for %s", thing))
	if err != nil {
		return tableType{}, err
	}
	return tableType{
		et:  et,
		lim: lim,
	}, nil
}

func (p *parser) ReadMemType(thing string) (memType, error) {
	lim, err := p.ReadLimits(fmt.Sprintf("limits for %s", thing))
	if err != nil {
		return memType{}, err
	}
	return memType{lim}, nil
}

func (p *parser) ReadGlobalType(thing string) (globalType, error) {
	t, err := p.ReadValType(thing)
	if err != nil {
		return globalType{}, err
	}
	mut, err := p.ReadByte(thing)
	if err != nil {
		return globalType{}, err
	}

	return globalType{
		mut: mut == 0x01,
		t:   t,
	}, nil
}

func (p *parser) ReadTagType(thing string) (uint32, error) {
	_, err := p.ReadByte(thing)
	if err != nil {
		return 0, err
	}
	idx, _, err := p.ReadU32(thing)
	return idx, err
}

func (p *parser) ReadValType(thing string) (valType, error) {
	at := p.cur

	t, err := p.ReadByte(thing)
	if err != nil {
		return valType{}, err
	}

	switch tc := typeCode(t); tc {
	case __rtNonNull, __rtNull:
		ht, err := p.ReadHeapType(thing)
		if err != nil {
			return valType{}, err
		}
		return valType{
			isRef: true,
			refType: refType{
				null: tc == __rtNull,
				ht:   ht,
			},
		}, nil
	default:
		if tc.IsNumType() || tc.IsVecType() {
			return valType{
				numOrVecType: tc,
			}, nil
		} else if tc.IsHeapType() {
			return valType{
				isRef: true,
				refType: refType{
					null: true,
					ht:   tc,
				},
			}, nil
		} else {
			return valType{}, fmt.Errorf("%s at offset %d: invalid valtype", thing, at)
		}
	}
}

func (p *parser) ReadRefType(thing string) (refType, error) {
	kind, err := p.PeekByte(thing)
	if err != nil {
		return refType{}, err
	}

	null := false
	if kind == 0x64 || kind == 0x63 {
		utils.Must1(p.ReadByte(thing))
		null = kind == 0x63
	}

	ht, err := p.ReadHeapType(thing)
	if err != nil {
		return refType{}, err
	}

	return refType{
		null: null,
		ht:   ht,
	}, nil
}

func (p *parser) ReadHeapType(thing string) (typeCode, error) {
	at := p.cur
	kind, n, err := p.ReadS64(thing)
	if err != nil {
		return 0, err
	}
	if kind < 0 && n != 1 {
		return 0, fmt.Errorf("%s at offset %d: invalid abstract heap type", thing, at)
	}
	ht := typeCode(kind)
	if !ht.IsHeapType() {
		return 0, fmt.Errorf("%s at offset %d: invalid heap type", thing, at)
	}
	return ht, nil
}

func (p *parser) ReadLimits(thing string) (limits, error) {
	flags, err := p.ReadByte("limits flags")
	if err != nil {
		return limits{}, err
	}

	min, _, err := p.ReadU64("limits min")
	if err != nil {
		return limits{}, err
	}

	lim := limits{min: min}
	if flags&0b001 > 0 {
		max, _, err := p.ReadU64("limits max")
		if err != nil {
			return limits{}, err
		}
		lim.hasMax = true
		lim.max = max
	}
	if flags&0b100 > 0 {
		lim.at = atI64
	}

	return lim, nil
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
