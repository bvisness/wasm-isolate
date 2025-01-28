package parser

type Void any

func __derefIfNotNil[T any](p *T) T {
	if p != nil {
		return *p
	}
	var zero T
	return zero
}
