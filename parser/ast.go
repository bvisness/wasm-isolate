package parser

type Local = *Phrase[Local_]

type Local_ struct {
	Ltype ValType
}

/*
type OTypeIdx OInt32
type OLocalIdx OInt32

type ONullKind int

const (
	KNoNull ONullKind = iota + 1
	KNull
)

type ONull interface {
	Kind() ONullKind
}

type SimpleONull struct {
	kind ONullKind
}

func (s SimpleONull) Kind() ONullKind {
	return s.kind
}

var _NoNull ONull = SimpleONull{KNoNull}
var _Null ONull = SimpleONull{KNull}

type OLimits struct {
	Min OInt64
	Max *OInt64
}

type ORefType struct {
	F0 ONull
	F1 HeapType
}

type HeapType interface {
	Kind() HeapTypeKind
}

type HeapTypeKind int

const (
	KAnyHT HeapTypeKind = iota + 1
	KNoneHT
	KEqHT
	KI31HT
	KStructHT
	KArrayHT
	KFuncHT
	KNoFuncHT
	KExnHT
	KNoExnHT
	KExternHT
	KNoExternHT
	KVarHT
	KDefHT
	KBotHT
)

type SimpleHeapType struct {
	kind HeapTypeKind
}

func (s SimpleHeapType) Kind() HeapTypeKind {
	return s.kind
}

type VarHeapType Var

func (v VarHeapType) Kind() HeapTypeKind {
	return KVarHT
}

var _AnyHT HeapType = SimpleHeapType{KAnyHT}

func _VarHT_1(v Var) HeapType {
	return VarHeapType(v)
}
*/
