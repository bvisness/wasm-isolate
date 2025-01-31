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
			res: NamedType("int32"),
		},
		{
			t:   "stream -> int32",
			res: Func{NamedType("stream"), NamedType("int32")},
		},
		{
			t:   "int -> stream -> int64",
			res: Func{NamedType("int"), Func{NamedType("stream"), NamedType("int64")}},
		},
		{
			t:   "(stream -> 'a) -> stream -> 'a",
			res: Func{Func{NamedType("stream"), NamedType("'a")}, Func{NamedType("stream"), NamedType("'a")}},
		},
		{
			t: "('a -> 'b) -> bool -> 'a -> 'b option",
			res: Func{
				Func{NamedType("'a"), NamedType("'b")},
				Func{
					NamedType("bool"),
					Func{
						NamedType("'a"),
						Cons{NamedType("'b"), NamedType("option")},
					},
				},
			},
		},
		{
			t: "stream -> local_idx * local' phrase",
			res: Func{
				NamedType("stream"),
				Tuple{NamedType("local_idx"), Cons{NamedType("local'"), NamedType("phrase")}},
			},
		},
		{
			t: "stream -> module_' * Custom.custom' phrase list",
			res: Func{
				NamedType("stream"),
				Tuple{NamedType("module_'"), Cons{NamedType("Custom.custom'"), NamedType("phrase"), NamedType("list")}},
			},
		},
		{
			t: "module_ -> string -> Custom.custom' phrase -> (module Custom.Section) list",
			res: Func{
				NamedType("module_"),
				Func{
					NamedType("string"),
					Func{
						Cons{NamedType("Custom.custom'"), NamedType("phrase")},
						Cons{NamedType("(module Custom.Section)"), NamedType("list")},
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
