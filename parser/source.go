package parser

// Manual translation of source.ml

type Pos struct {
	file   string
	line   int
	column int
}

type Region struct {
	left  *Pos
	right *Pos
}

type Phrase[T any] struct {
	at *Region
	it T
}
