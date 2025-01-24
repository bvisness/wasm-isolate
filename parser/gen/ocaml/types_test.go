package ocaml

import (
	"testing"

	"github.com/stretchr/testify/assert"
)

func TestParseType(t *testing.T) {
	type Case struct {
		t   string
		res Type
	}
	cases := []Case{
		{
			t:   "int32",
			res: SimpleType("int32"),
		},
		{
			t:   "stream -> int32",
			res: Func{SimpleType("stream"), SimpleType("int32")},
		},
		{
			t:   "int -> stream -> int64",
			res: Func{SimpleType("int"), Func{SimpleType("stream"), SimpleType("int64")}},
		},
		{
			t:   "(stream -> 'a) -> stream -> 'a",
			res: Func{Func{SimpleType("stream"), SimpleType("'a")}, Func{SimpleType("stream"), SimpleType("'a")}},
		},
		{
			t: "('a -> 'b) -> bool -> 'a -> 'b option",
			res: Func{
				Func{SimpleType("'a"), SimpleType("'b")},
				Func{
					SimpleType("bool"),
					Func{
						SimpleType("'a"),
						Cons{SimpleType("'b"), SimpleType("option")},
					},
				},
			},
		},
		{
			t: "stream -> local_idx * local' phrase",
			res: Func{
				SimpleType("stream"),
				Tuple{SimpleType("local_idx"), Cons{SimpleType("local'"), SimpleType("phrase")}},
			},
		},
		{
			t: "stream -> module_' * Custom.custom' phrase list",
			res: Func{
				SimpleType("stream"),
				Tuple{SimpleType("module_'"), Cons{SimpleType("Custom.custom'"), SimpleType("phrase"), SimpleType("list")}},
			},
		},
		{
			t: "module_ -> string -> Custom.custom' phrase -> (module Custom.Section) list",
			res: Func{
				SimpleType("module_"),
				Func{
					SimpleType("string"),
					Func{
						Cons{SimpleType("Custom.custom'"), SimpleType("phrase")},
						Cons{SimpleType("(module Custom.Section)"), SimpleType("list")},
					},
				},
			},
		},
	}

	for _, c := range cases {
		t.Run(c.t, func(t *testing.T) {
			assert.Equal(t, c.res, ParseType(c.t, nil))
		})
	}
}
