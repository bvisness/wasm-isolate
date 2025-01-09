package isolate

import (
	"errors"
	"io"
	"slices"

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

	out.Write([]byte{0, 'a', 's', 'm'})
	out.Write([]byte{1, 0, 0, 0})

	var numImportedFuncs uint32
	var newFuncs []wasmFunc
	var old2new map[uint32]int = make(map[uint32]int)

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

		bodyStart := p.cur
		body, err := p.ReadN("section contents", int(sectionSize))
		if err != nil {
			return err
		}

		p := newParserFromBytes(body, bodyStart)

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
					numImportedFuncs++
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

			// Pass the section through as is
			out.Write([]byte{sectionId})
			out.Write(leb128.EncodeU64(uint64(sectionSize)))
			out.Write(body)
		case 3: // function section
			numFuncs, _, err := p.ReadU32("num funcs")
			if err != nil {
				return err
			}
			for i := range numFuncs {
				t, _, err := p.ReadU32("function type")
				if err != nil {
					return err
				}

				funcIdx := numImportedFuncs + i
				if slices.Contains(funcs, int(funcIdx)) {
					old2new[funcIdx] = len(newFuncs)
					newFuncs = append(newFuncs, wasmFunc{
						typeIndex: t,
					})
				}
			}

			out.Write([]byte{sectionId})
			var newBody []byte
			newBody = append(newBody, leb128.EncodeU64(uint64(len(newFuncs)))...)
			for _, f := range newFuncs {
				newBody = append(newBody, leb128.EncodeU64(uint64(f.typeIndex))...)
			}
			out.Write(leb128.EncodeU64(uint64(len(newBody))))
			out.Write(newBody)
		// case 9: // element section
		// TODO: Parse segments of funcref to make sure we don't strip declared funcs
		case 7, 8: // export and start sections
			// These sections are unnecessary and can be skipped.
		case 10: // code section
			numEntries, _, err := p.ReadU32("num code entries")
			if err != nil {
				return err
			}

			for i := range numEntries {
				size, _, err := p.ReadU32("code entry size")
				if err != nil {
					return err
				}
				body, err := p.ReadN("code entry body", int(size))
				if err != nil {
					return err
				}

				funcIdx := numImportedFuncs + i
				if slices.Contains(funcs, int(funcIdx)) {
					newFuncs[old2new[funcIdx]].body = body
				}
			}

			out.Write([]byte{sectionId})
			var newBody []byte
			newBody = append(newBody, leb128.EncodeU64(uint64(len(newFuncs)))...)
			for _, f := range newFuncs {
				newBody = append(newBody, leb128.EncodeU64(uint64(len(f.body)))...)
				newBody = append(newBody, f.body...)
			}
			out.Write(leb128.EncodeU64(uint64(len(newBody))))
			out.Write(newBody)
		default:
			out.Write([]byte{sectionId})
			out.Write(leb128.EncodeU64(uint64(sectionSize)))
			out.Write(body)
		}
	}

	return nil
}

type wasmFunc struct {
	typeIndex uint32
	body      []byte
}
