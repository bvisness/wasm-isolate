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

type TypeDefs map[string]Type

type TypeKind int

const (
	TFunc TypeKind = iota + 1
	TTuple
	TCons
	TVariants
	TAlias
	TModule
	TSimple
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

type Alias struct {
	Name string
	Type Type
}

func (t Alias) String() string {
	return t.Name
}

func (t Alias) Kind() TypeKind {
	return TAlias
}

type ModuleType struct {
	Modules []string
	Type    Type
}

func (t ModuleType) String() string {
	return fmt.Sprintf("%s.%s", strings.Join(t.Modules, "."), t.Type)
}

func (t ModuleType) Kind() TypeKind {
	return TModule
}

type SimpleType string

func (t SimpleType) String() string {
	return string(t)
}

func (t SimpleType) Kind() TypeKind {
	return TSimple
}

func ParseType(t string, typeDefs TypeDefs) Type {
	tree := parser.Parse([]byte(t), nil)
	defer tree.Close()

	if tree.RootNode().HasError() {
		panic(fmt.Sprintf("type has error: %s", t))
	}

	return parseTypeNode(tree.RootNode(), t, typeDefs)
}

func parseTypeNode(n *tree_sitter.Node, t string, typeDefs TypeDefs) Type {
	fmt.Fprintf(os.Stderr, "type node %s: %s\n", n.GrammarName(), n.Utf8Text([]byte(t)))
	fmt.Fprintf(os.Stderr, "  %s\n", n.ToSexp())

	switch n.GrammarName() {
	case "type", "parenthesized_type":
		return parseTypeNode(n.NamedChild(0), t, typeDefs)
	case "package_type", "type_variable", "type_constructor", "_lowercase_identifier":
		name := n.Utf8Text([]byte(t))
		if def, ok := typeDefs[name]; ok {
			return def
		}
		return SimpleType(name)
	case "type_constructor_path":
		var modules []string
		for i := range n.NamedChildCount() - 1 {
			modules = append(modules, n.NamedChild(i).Utf8Text([]byte(t)))
		}
		ty := parseTypeNode(n.NamedChild(n.NamedChildCount()-1), t, typeDefs)
		if len(modules) > 0 {
			return ModuleType{
				Modules: modules,
				Type:    ty,
			}
		} else {
			return ty
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
