package isolate

import (
	"bytes"
	"errors"
	"fmt"
	"io"
	"regexp"

	"github.com/bvisness/wasm-isolate/leb128"
	"github.com/bvisness/wasm-isolate/utils"
)

// TODO:
// - Preserve functions that are called (via various opcodes)
// - Preserve functions that are reffed

func Isolate(wasm io.Reader, out io.Writer, funcsToKeep []int) error {
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
	var declaredFuncs []uint32
	var outFuncs []wasmFunc
	var sections []section

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
		asPlainSection := plainSection{
			id:   sectionId,
			body: body,
		}

		switch sectionId {
		case 0: // custom section
			secName, err := p.ReadName("custom section name")
			if err != nil {
				continue
			}
			if secName == "name" {
				// TODO: Regenerate the name section
				// // Pass the section through
				// out.Write([]byte{sectionId})
				// out.Write(leb128.EncodeU64(uint64(sectionSize)))
				// out.Write(body)
			} else {
				// Ignore it
			}
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
					t, _, err := p.ReadU32("type of imported function")
					if err != nil {
						return err
					}
					outFuncs = append(outFuncs, wasmFunc{
						typeIndex: t,
					})
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

			sections = append(sections, &asPlainSection)
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
				utils.Assert(len(outFuncs) == int(funcIdx), "didn't track function indices correctly")
				outFuncs = append(outFuncs, wasmFunc{
					typeIndex: t,
				})
			}

			sections = append(sections, &functionSection{
				funcs:      &outFuncs,
				startIndex: &numImportedFuncs,
			})
		case 9: // element section
			numSegments, _, err := p.ReadU32("num elem segments")
			if err != nil {
				return err
			}

			for range numSegments {
				flags, _, err := p.ReadU32("elem segment flags")
				if err != nil {
					return err
				}
				active := !(flags&0b001 != 0)
				activeHasTableIndex := flags&0b010 != 0
				exprEncoding := flags&0b100 != 0
				if exprEncoding {
					return fmt.Errorf("elem segment at offset %d: the expression encoding is not supported", p.cur)
				}

				if active {
					if activeHasTableIndex {
						_, _, err := p.ReadU32("elem segment table index")
						if err != nil {
							return err
						}
					}

					_, err := p.ReadExpr("elem segment offset expression")
					if err != nil {
						return err
					}
				}

				if exprEncoding {
					// TODO (impossible due to earlier guard at the moment)
				} else {
					if !(active && !activeHasTableIndex) {
						err := p.Expect("elem segment kind", []byte{0x00})
						if err != nil {
							return err
						}
					}

					numElems, _, err := p.ReadU32("elem segment num elems")
					if err != nil {
						return err
					}
					for range numElems {
						idx, _, err := p.ReadU32("elem segment func index")
						if err != nil {
							return err
						}
						declaredFuncs = append(declaredFuncs, idx)
					}
				}
			}

			sections = append(sections, &asPlainSection)
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
				outFuncs[funcIdx].body = body
			}

			sections = append(sections, &codeSection{
				funcs:      &outFuncs,
				startIndex: &numImportedFuncs,
			})
		default:
			sections = append(sections, &asPlainSection)
		}
	}

	// Mark functions that we are going to keep (including imports)
	// TODO: Do not keep all imports :)
	{
		for funcIdx := range numImportedFuncs {
			outFuncs[funcIdx].keep = true
		}
		for _, funcIdx := range funcsToKeep {
			outFuncs[funcIdx].keep = true
		}
		for _, funcIdx := range declaredFuncs {
			outFuncs[funcIdx].keep = true
		}
	}

	// Mark all functions with their new index
	{
		var nextNewFuncIndex uint32 = 0
		for i := range outFuncs {
			if outFuncs[i].keep {
				outFuncs[i].newIndex = nextNewFuncIndex
				nextNewFuncIndex++
			}
		}
	}

	// Replace function indices in function bodies
	// TODO: Replace this hack with a full expression parse.
	// TODO: It already doesn't work :( :( :( :( :(
	{
		for i := numImportedFuncs; int(i) < len(outFuncs); i++ {
			f := &outFuncs[i]
			if !f.keep {
				continue
			}

			// TODO: In the future, we will probably have to parse the function body
			// earlier in order to discover which other functions it calls (and which
			// should therefore be kept alive). When we do that, we should track the
			// locations of function indices so that we can avoid doing another parse
			// here.

			var newBody bytes.Buffer
			nextChunkStart := 0
			instrs := reInstrWithFuncIndex.FindAllIndex(f.body, -1)
			for _, instrBounds := range instrs {
				// Write the body up to the func index
				chunk := f.body[nextChunkStart:instrBounds[1]]
				utils.Must1(newBody.Write(chunk))

				// Parse the old func index and look up the new one
				oldFuncIndex, n, err := leb128.DecodeU64(bytes.NewBuffer(f.body[instrBounds[1]+1:]))
				if err != nil {
					return fmt.Errorf("invalid func index while relocating: %w", err)
				}
				calledFunc := &outFuncs[oldFuncIndex]

				// Write the new func index
				utils.Assert(calledFunc.keep, "any function we call should have been kept, but func %d was not (in func %d's body)", oldFuncIndex, i)
				utils.Must1(newBody.Write(leb128.EncodeU64(uint64(calledFunc.newIndex))))

				// Prep the next chunk
				nextChunkStart = instrBounds[1] + n
			}
			// Write the remainder of the body after the final type index
			utils.Must1(newBody.Write(f.body[nextChunkStart:]))

			// Swap the body!
			f.body = newBody.Bytes()
		}
	}

	// Actually swap all the data used for output
	{
		var newOutFuncs []wasmFunc
		for i, f := range outFuncs {
			if i < int(numImportedFuncs) {
				// imported
				newOutFuncs = append(newOutFuncs, f)
			} else {
				// defined
				if f.keep {
					// TODO: Actually relocate things
					newOutFuncs = append(newOutFuncs, f)
				}
			}
		}
		outFuncs = newOutFuncs
	}

	for _, sec := range sections {
		sec.WriteSection(out)
	}

	return nil
}

var reInstrWithFuncIndex = regexp.MustCompile(`\x10|\x12|\xD2`)

type wasmFunc struct {
	typeIndex uint32
	body      []byte
	keep      bool
	newIndex  uint32
}

type section interface {
	WriteSection(out io.Writer)
}

type plainSection struct {
	id   byte
	body []byte
}

func (s *plainSection) WriteSection(out io.Writer) {
	out.Write([]byte{s.id})
	out.Write(leb128.EncodeU64(uint64(len(s.body))))
	out.Write(s.body)
}

type functionSection struct {
	funcs      *[]wasmFunc
	startIndex *uint32
}

func (s *functionSection) WriteSection(out io.Writer) {
	funcs := (*s.funcs)[*s.startIndex:]

	var body []byte
	body = append(body, leb128.EncodeU64(uint64(len(funcs)))...)
	for _, f := range funcs {
		body = append(body, leb128.EncodeU64(uint64(f.typeIndex))...)
	}

	out.Write([]byte{3})
	out.Write(leb128.EncodeU64(uint64(len(body))))
	out.Write(body)
}

type codeSection struct {
	funcs      *[]wasmFunc
	startIndex *uint32
}

func (s *codeSection) WriteSection(out io.Writer) {
	funcs := (*s.funcs)[*s.startIndex:]

	var body []byte
	body = append(body, leb128.EncodeU64(uint64(len(funcs)))...)
	for _, f := range funcs {
		body = append(body, leb128.EncodeU64(uint64(len(f.body)))...)
		body = append(body, f.body...)
	}

	out.Write([]byte{10})
	out.Write(leb128.EncodeU64(uint64(len(body))))
	out.Write(body)
}
