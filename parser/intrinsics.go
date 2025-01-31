package parser

func __derefIfNotNil[T any](p *T) T {
	if p != nil {
		return *p
	}
	var zero T
	return zero
}

// Void is defined as an empty variant type, so we follow that pattern here.

type OVoidKind int

type OVoid interface {
	Kind() OVoidKind
}
