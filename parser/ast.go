package parser

type Local = *Phrase[Local_]

type Local_ struct {
	Ltype ValType
}
