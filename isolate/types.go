package isolate

type memType struct {
	lim limits
}

type tableType struct {
	et  refType
	lim limits
}

type globalType struct {
	mut bool
	t   valType
}

type addressType int

const (
	atI32 addressType = iota
	atI64
)

type limits struct {
	at       addressType
	min, max uint64
	hasMax   bool
}

type valType struct {
	isRef        bool
	numOrVecType typeCode
	refType      refType
}

func (vt valType) IsNumType() bool {
	return !vt.isRef && vt.numOrVecType.IsNumType()
}

func (vt valType) IsVecType() bool {
	return !vt.isRef && vt.numOrVecType.IsVecType()
}

func (vt valType) IsRefType() bool {
	return vt.isRef
}

func (vt valType) NumType() typeCode {
	if !vt.IsNumType() {
		panic("valtype was not a numtype")
	}
	return vt.numOrVecType
}

func (vt valType) VecType() typeCode {
	if !vt.IsVecType() {
		panic("valtype was not a vectype")
	}
	return vt.numOrVecType
}

func (vt valType) RefType() refType {
	if !vt.IsRefType() {
		panic("valtype was not a reftype")
	}
	return vt.refType
}

type refType struct {
	null bool
	ht   typeCode // may be an abstract heap type or a concrete one, depending on sign
}

type typeCode int

const (
	// The hex bytes in here refer to the number's encoding in SLEB128.

	// numtype
	nt__last  typeCode = ntI32
	ntI32     typeCode = -1 // 0x7F
	ntI64     typeCode = -2 // 0x7E
	ntF32     typeCode = -3 // 0x7D
	ntF64     typeCode = -4 // 0x7C
	nt__first typeCode = ntF64

	// vectype
	vt__last  typeCode = vtV128
	vtV128    typeCode = -5 // 0x7B
	vt__first typeCode = vtV128

	// heaptype (abstract, because positive values mean concrete type index)
	ht__last   typeCode = htNoExn
	htNoExn    typeCode = -12 // 0x74
	htNoFunc   typeCode = -13 // 0x73
	htNoExtern typeCode = -14 // 0x72
	htNone     typeCode = -15 // 0x71
	htFunc     typeCode = -16 // 0x70
	htExtern   typeCode = -17 // 0x6F
	htAny      typeCode = -18 // 0x6E
	htEq       typeCode = -19 // 0x6D
	htI31      typeCode = -20 // 0x6C
	htStruct   typeCode = -21 // 0x6B
	htArray    typeCode = -22 // 0x6A
	htExn      typeCode = -23 // 0x69
	ht__first  typeCode = htExn

	// Sentinel bytes indicating that a ref type's heap type follows.
	__rtNonNull typeCode = -28 // 0x64
	__rtNull    typeCode = -29 // 0x63
)

func (tc typeCode) IsNumType() bool {
	return nt__first <= tc && tc <= nt__last
}

func (tc typeCode) IsVecType() bool {
	return vt__first <= tc && tc <= vt__last
}

func (tc typeCode) IsHeapType() bool {
	return tc.IsAbstractHeapType() || tc.IsConcreteHeapType()
}

func (tc typeCode) IsAbstractHeapType() bool {
	return ht__first <= tc && tc <= ht__last
}

func (tc typeCode) IsConcreteHeapType() bool {
	return tc > 0
}
