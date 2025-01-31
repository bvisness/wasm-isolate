package ocaml

import (
	"fmt"
	"os"
	"strings"

	tree_sitter "github.com/tree-sitter/go-tree-sitter"
	tree_sitter_ocaml "github.com/tree-sitter/tree-sitter-ocaml/bindings/go"
)

var ocaml = tree_sitter.NewLanguage(tree_sitter_ocaml.LanguageOCamlType())
var parser = tree_sitter.NewParser()

func init() {
	parser.SetLanguage(ocaml)
}

type TypeKind int

const (
	TFunc TypeKind = iota + 1
	TTuple
	TCons
	TVariants
	TRecord
	TTypeDef
	TNamed
)

type Type interface {
	fmt.Stringer
	Kind() TypeKind
}

type Func struct {
	In  Type
	Out Type
}

func (f Func) String() string {
	return fmt.Sprintf("%s -> %s", f.In, f.Out)
}

func (f Func) Kind() TypeKind {
	return TFunc
}

func (f Func) GetArgType(i int) Type {
	funcType := f
	for range i {
		funcType = funcType.Out.(Func)
	}
	return funcType.In
}

func (f Func) GetTypeAfterApplyingArgs(numArgs int) Type {
	var res Type = f
	for range numArgs {
		res = res.(Func).Out
	}
	return res
}

type Tuple []Type

func (t Tuple) String() string {
	strs := make([]string, len(t))
	for i, child := range t {
		strs[i] = child.String()
	}
	return strings.Join(strs, " * ")
}

func (t Tuple) Kind() TypeKind {
	return TTuple
}

type Cons []Type

func (t Cons) String() string {
	strs := make([]string, len(t))
	for i, child := range t {
		strs[i] = child.String()
	}
	return strings.Join(strs, " ")
}

func (t Cons) Kind() TypeKind {
	return TCons
}

type Variant struct {
	Name string
	Type *Type
}

func (v Variant) String() string {
	return v.Name
}

type Variants []Variant

func (t Variants) String() string {
	strs := make([]string, len(t))
	for i, child := range t {
		strs[i] = child.String()
	}
	return strings.Join(strs, " | ")
}

func (t Variants) Kind() TypeKind {
	return TVariants
}

type Record []RecordField

type RecordField struct {
	Name string
	Type Type
}

func (t Record) String() string {
	res := "{"
	for i, f := range t {
		if i > 0 {
			res += "; "
		}
		res += fmt.Sprintf("%s : %s", f.Name, f.Type)
	}
	res += "}"
	return res
}

func (t Record) Kind() TypeKind {
	return TRecord
}

type TypeDef struct {
	Modules []string
	Name    string
	Type    Type
}

func (t TypeDef) String() string {
	if len(t.Modules) > 0 {
		return strings.Join(t.Modules, ".") + "." + t.Name
	} else {
		return t.Name
	}
}

func (t TypeDef) Kind() TypeKind {
	return TTypeDef
}

type NamedType struct {
	Modules []string
	Name    string
}

func (t NamedType) String() string {
	if len(t.Modules) > 0 {
		return strings.Join(t.Modules, ".") + "." + t.Name
	} else {
		return t.Name
	}
}

func (t NamedType) Kind() TypeKind {
	return TNamed
}

func ParseType(t string, typeDefs map[string]TypeDef) Type {
	tree := parser.Parse([]byte(t), nil)
	defer tree.Close()

	if tree.RootNode().HasError() {
		panic(fmt.Sprintf("type has error: %s", t))
	}

	return parseTypeNode(tree.RootNode(), t, typeDefs)
}

func parseTypeNode(n *tree_sitter.Node, t string, typeDefs map[string]TypeDef) Type {
	// fmt.Fprintf(os.Stderr, "type node %s: %s\n", n.GrammarName(), n.Utf8Text([]byte(t)))
	// fmt.Fprintf(os.Stderr, "  %s\n", n.ToSexp())

	switch n.GrammarName() {
	case "type", "parenthesized_type":
		return parseTypeNode(n.NamedChild(0), t, typeDefs)
	case "package_type", "type_variable", "type_constructor", "_lowercase_identifier":
		name := n.Utf8Text([]byte(t))
		if def, ok := typeDefs[name]; ok {
			return def
		}
		return NamedType{nil, name}
	case "type_constructor_path":
		ty := parseTypeNode(n.NamedChild(n.NamedChildCount()-1), t, typeDefs)
		switch ty := ty.(type) {
		case NamedType:
			for i := range n.NamedChildCount() - 1 {
				ty.Modules = append(ty.Modules, n.NamedChild(i).Utf8Text([]byte(t)))
			}
			return ty
		case TypeDef:
			return ty
		default:
			panic(fmt.Sprintf("how can you even have a %+v in a type_constructor_path", ty))
		}

	case "constructed_type":
		var cons Cons
		t1 := parseTypeNode(n.NamedChild(0), t, typeDefs)
		t2 := parseTypeNode(n.NamedChild(1), t, typeDefs)
		if t1cons, ok := t1.(Cons); ok {
			cons = append(cons, t1cons...)
		} else {
			cons = append(cons, t1)
		}
		if t2cons, ok := t2.(Cons); ok {
			cons = append(cons, t2cons...)
		} else {
			cons = append(cons, t2)
		}
		return cons
	case "function_type":
		return Func{
			In:  parseTypeNode(n.NamedChild(0), t, typeDefs),
			Out: parseTypeNode(n.NamedChild(1), t, typeDefs),
		}
	case "tuple_type":
		var tup Tuple
		t1 := parseTypeNode(n.NamedChild(0), t, typeDefs)
		t2 := parseTypeNode(n.NamedChild(1), t, typeDefs)
		if t1tup, ok := t1.(Tuple); ok {
			tup = append(tup, t1tup...)
		} else {
			tup = append(tup, t1)
		}
		if t2tup, ok := t2.(Tuple); ok {
			tup = append(tup, t2tup...)
		} else {
			tup = append(tup, t2)
		}
		return tup
	default:
		fmt.Fprintf(os.Stderr, "%s\n", n.ToSexp())
		panic(fmt.Sprintf("unknown type node %s", n.GrammarName()))
	}
}
