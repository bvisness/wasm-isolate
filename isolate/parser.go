package isolate

import (
	"bufio"
	"bytes"
	"fmt"
	"io"
	"math"

	"github.com/bvisness/wasm-isolate/leb128"
	"github.com/bvisness/wasm-isolate/utils"
)

type parser struct {
	r   *bufio.Reader
	cur int

	record   bool
	recorded []byte
}

func newParser(r io.Reader) parser {
	return parser{
		r:   bufio.NewReader(r),
		cur: 0,
	}
}

func newParserFromBytes(b []byte, at int) parser {
	return parser{
		r:   bufio.NewReader(bytes.NewBuffer(b)),
		cur: at,
	}
}

func (p *parser) StartRecording() {
	p.record = true
	p.recorded = nil
}

func (p *parser) StopRecording() []byte {
	p.record = false
	return p.recorded
}

func (p *parser) ReadN(thing string, n int) ([]byte, error) {
	at := p.cur
	bytes := make([]byte, n)
	nRead, err := io.ReadFull(p.r, bytes)
	if err != nil {
		return nil, fmt.Errorf("%s at offset %d: %w", thing, at, err)
	}
	p.cur += nRead
	if p.record {
		p.recorded = append(p.recorded, bytes...)
	}
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
	if p.record {
		p.recorded = append(p.recorded, b[0])
	}
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

func (p *parser) ReadF32(thing string) (float32, error) {
	b, err := p.ReadN(thing, 4)
	if err != nil {
		return 0, err
	}
	bits := uint32(b[0])<<0 |
		uint32(b[1])<<8 |
		uint32(b[2])<<16 |
		uint32(b[3])<<24
	return math.Float32frombits(bits), nil
}

func (p *parser) ReadF64(thing string) (float64, error) {
	b, err := p.ReadN(thing, 8)
	if err != nil {
		return 0, err
	}
	bits := uint64(b[0])<<0 |
		uint64(b[1])<<8 |
		uint64(b[2])<<16 |
		uint64(b[3])<<24 |
		uint64(b[4])<<32 |
		uint64(b[5])<<40 |
		uint64(b[6])<<48 |
		uint64(b[7])<<56
	return math.Float64frombits(bits), nil
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

func (p *parser) ReadExpr(thing string) ([]byte, error) {
	p.StartRecording()
	defer p.StopRecording()

	depth := 0

instrs:
	for {
		b1, err := p.ReadByte(thing)
		if err != nil {
			return nil, err
		}

		switch b1 {
		case 0x0B: // end
			if depth == 0 {
				break instrs
			}
			depth -= 1
		case 0x41: // i32.const n
			_, _, err := p.ReadU32(fmt.Sprintf("i32.const in %s", thing))
			if err != nil {
				return nil, err
			}
		case 0x42: // i64.const n
			_, _, err := p.ReadU64(fmt.Sprintf("i64.const in %s", thing))
			if err != nil {
				return nil, err
			}
		case 0x43: // f32.const z
			_, err := p.ReadF32(fmt.Sprintf("f32.const in %s", thing))
			if err != nil {
				return nil, err
			}
		case 0x44: // f64.const z
			_, err := p.ReadF64(fmt.Sprintf("f64.const in %s", thing))
			if err != nil {
				return nil, err
			}

		case 0x6A: // i32.add
		case 0x6B: // i32.sub
		case 0x6C: // i32.mul

		case 0x7C: // i64.add
		case 0x7D: // i64.sub
		case 0x7E: // i64.mul

		// case 0xD0: // ref.null

		default:
			return nil, fmt.Errorf("%s at offset %d: unknown opcode %x", thing, p.cur-1, b1)
		}
	}

	return p.recorded, nil
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
