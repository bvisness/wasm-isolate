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

type Module struct {
	ParentModules []string
	Name          string
	TypeDefs      map[string]Def
	ValueDefs     map[string]Def
}

func NewModule(namespace []string, name string) *Module {
	m := &Module{
		ParentModules: namespace,
		Name:          name,
		ValueDefs:     map[string]Def{},
	}
	m.TypeDefs = map[string]Def{
		"bool":   Def{m.Namespace(), Primitive("bool")},
		"string": Def{m.Namespace(), Primitive("string")},
		"int":    Def{m.Namespace(), Primitive("OInt")},
		"int32":  Def{m.Namespace(), Primitive("OInt32")},
		"int64":  Def{m.Namespace(), Primitive("OInt64")},

		"list":   Def{m.Namespace(), Primitive("list")},
		"option": Def{m.Namespace(), Primitive("option")},
	}
	return m
}

func (m Module) Clone() *Module {
	res := &Module{
		ParentModules: m.ParentModules,
		Name:          m.Name,
		TypeDefs:      map[string]Def{},
		ValueDefs:     map[string]Def{},
	}
	for name, def := range m.TypeDefs {
		res.TypeDefs[name] = def
	}
	for name, def := range m.ValueDefs {
		res.ValueDefs[name] = def
	}
	return res
}

func (m Module) Namespace() Namespace {
	return append(m.ParentModules, m.Name)
}

type Namespace []string

func (n Namespace) String() string {
	return strings.Join(n, ".")
}

type Def struct {
	Namespace []string
	Type      Type
}

type TypeKind int

const (
	TFunc TypeKind = iota + 1
	TTuple
	TCons
	TVariants
	TRecord
	TTypeDef
	TIdentifier
	TPrimitive
	TTypeVar
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

type Cons struct {
	Types []Type
	Base  Type // Not TypeDef here because we need to allow for built-in primitives
}

func (t Cons) String() string {
	strs := make([]string, len(t.Types)+1)
	for i, child := range t.Types {
		strs[i] = child.String()
	}
	strs[len(t.Types)] = t.Base.String()
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

type Identifier struct {
	Modules []string
	Name    string
}

func (t Identifier) String() string {
	if len(t.Modules) > 0 {
		return strings.Join(t.Modules, ".") + "." + t.Name
	} else {
		return t.Name
	}
}

func (t Identifier) Kind() TypeKind {
	return TIdentifier
}

type TypeDef struct {
	// TODO: Do we need modules in this any more? Now we're tracking a module path on all
	// definitions, so this seems redundant and likely just wrong.
	Identifier
	Type     Type
	TypeVars []TypeVar
}

func (t TypeDef) String() string {
	return t.Identifier.String()
}

func (t TypeDef) Kind() TypeKind {
	return TTypeDef
}

type Primitive string

func (t Primitive) String() string {
	return string(t)
}

func (t Primitive) Kind() TypeKind {
	return TPrimitive
}

type TypeVar string

func (t TypeVar) String() string {
	return string(t)
}

func (t TypeVar) Kind() TypeKind {
	return TTypeVar
}

func ParseType(t string, currentModule *Module) Type {
	tree := parser.Parse([]byte(t), nil)
	defer tree.Close()

	if tree.RootNode().HasError() {
		panic(fmt.Sprintf("type has error: \"%s\" %s", t, tree.RootNode().ToSexp()))
	}

	return parseTypeNode(tree.RootNode(), t, currentModule)
}

func parseTypeNode(n *tree_sitter.Node, t string, currentModule *Module) Type {
	// fmt.Fprintf(os.Stderr, "type node %s: %s\n", n.GrammarName(), n.Utf8Text([]byte(t)))
	// fmt.Fprintf(os.Stderr, "  %s\n", n.ToSexp())

	switch n.GrammarName() {
	case "type", "parenthesized_type":
		return parseTypeNode(n.NamedChild(0), t, currentModule)
	case "package_type", "type_variable", "type_constructor", "_lowercase_identifier":
		name := n.Utf8Text([]byte(t))
		if def, ok := currentModule.TypeDefs[name]; ok {
			return def.Type
			// return def.Type.(TypeDef) // assert
		}
		return Identifier{nil, name}
	case "type_constructor_path":
		ty := parseTypeNode(n.NamedChild(n.NamedChildCount()-1), t, currentModule)
		switch ty := ty.(type) {
		case Identifier:
			for i := range n.NamedChildCount() - 1 {
				ty.Modules = append(ty.Modules, n.NamedChild(i).Utf8Text([]byte(t)))
			}
			return ty
		case Primitive, TypeDef:
			return ty
		default:
			panic(fmt.Sprintf("how can you even have a %+v in a type_constructor_path (within %s)", ty, t))
		}

	case "constructed_type":
		var cons Cons
		for i := range n.NamedChildCount() - 1 {
			cons.Types = append(cons.Types, parseTypeNode(n.NamedChild(i), t, currentModule))
		}
		cons.Base = parseTypeNode(n.NamedChild(n.NamedChildCount()-1), t, currentModule)
		return cons
	case "function_type":
		return Func{
			In:  parseTypeNode(n.NamedChild(0), t, currentModule),
			Out: parseTypeNode(n.NamedChild(1), t, currentModule),
		}
	case "tuple_type":
		var tup Tuple
		t1 := parseTypeNode(n.NamedChild(0), t, currentModule)
		t2 := parseTypeNode(n.NamedChild(1), t, currentModule)
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
