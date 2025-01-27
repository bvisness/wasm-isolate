package parser

import "math"

// Manual translation of source.ml

type Pos struct {
	file   string
	line   OInt
	column OInt
}

type Region struct {
	left  *Pos
	right *Pos
}

type Phrase[T any] struct {
	at *Region
	it T
}

func _operatorAtAt_2[T any](x T, region *Region) *Phrase[T] {
	return &Phrase[T]{
		it: x,
		at: region,
	}
}

var _no_pos = &Pos{
	file:   "",
	line:   0,
	column: 0,
}

var _no_region = &Region{
	left:  _no_pos,
	right: _no_pos,
}

func _all_region(file string) *Region {
	return &Region{
		left:  &Pos{file, 0, 0},
		right: &Pos{file, math.MaxInt, math.MaxInt},
	}
}
