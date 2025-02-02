package main

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"strings"

	"github.com/bvisness/wasm-isolate/parser/gen/ocaml"
	"github.com/bvisness/wasm-isolate/utils"
	"github.com/spf13/cobra"
	tree_sitter "github.com/tree-sitter/go-tree-sitter"
	tree_sitter_ocaml "github.com/tree-sitter/tree-sitter-ocaml/bindings/go"
)

var specpath = filepath.Join("gen", "spec")

type File struct {
	Path []string
	Skip []string

	ModuleName string

	AllFuncs             bool
	SkipTypes, SkipFuncs bool
}

var files = []File{
	{
		Path:       []string{"interpreter", "syntax", "types.ml"},
		ModuleName: "Types",
		SkipFuncs:  true,
	},
	{
		Path:       []string{"interpreter", "syntax", "pack.ml"},
		ModuleName: "Pack",
	},
	{
		Path:       []string{"interpreter", "util", "source.ml"},
		ModuleName: "Source",
		Skip:       []string{"phrase"},
	},
	{
		Path:       []string{"interpreter", "syntax", "ast.ml"},
		Skip:       []string{"void"},
		ModuleName: "Ast",
		SkipFuncs:  true,
	},
	{
		Path:       []string{"interpreter", "syntax", "operators.ml"},
		ModuleName: "Operators",
		AllFuncs:   true,
	},
	{
		Path:       []string{"interpreter", "binary", "decode.ml"},
		ModuleName: "Decode",
	},
}

var toTranslate = []string{
	// Generic values
	"bit", "byte", "word16", "word32", "word64",
	"uN", "sN", "u32", "u64", "s7", "s32", "s33", "s64", "f32", "f64", "v128",
	"len32", "string",

	// Types
	"zero", "var",
	"mutability", "var_type", "num_type", "vec_type", "heap_type", "ref_type",
	"val_type", "result_type", "pack_type", "storage_type", "field_type",
	"struct_type", "array_type", "func_type", "str_type", "sub_type", "rec_type",
	"limits", "table_type", "global_type", "tag_type",

	// Instructions
	"op", "end_", "memop",
	"block_type", "local",
	"instr",
}

var outFile *os.File
var tmpCount int

var lspClient *ocaml.Client
var ocamlParser *tree_sitter.Parser
var modules = make(map[string]*ocaml.Module)

func ocaml2go(t ocaml.Type, currentModule *ocaml.Module) string {
	base := map[string]string{
		"Utf8.unicode": "string",

		"F32.t":  "float32",
		"F64.t":  "float64",
		"V128.t": "V128",

		"address": "OInt64",

		"stream": "*Stream",
	}

	if goType, ok := base[t.String()]; ok {
		return goType
	} else if existing, ok := currentModule.Defs[t.String()]; ok {
		// TODO: This logic is probably wrong now that t.String() has modules in it, right?
		switch existing := existing.(type) {
		case ocaml.TypeDef:
			return typeName(existing.Modules, existing.Name)
		default:
			return ocaml2go(existing, &ocaml.Module{})
		}
	} else if t.String()[0] == '\'' {
		return fmt.Sprintf("any /* %s */", t)
	} else if asTypeDef, ok := t.(ocaml.TypeDef); ok {
		return typeName(asTypeDef.Modules, asTypeDef.Name)
	} else if asIdent, ok := t.(ocaml.Identifier); ok {
		return typeName(asIdent.Modules, asIdent.Name)
	} else if asPrimitive, ok := t.(ocaml.Primitive); ok {
		return string(asPrimitive)
	} else if asCons, ok := t.(ocaml.Cons); ok {
		last := asCons[len(asCons)-1]
		if last.String() == "Source.phrase" {
			// Temporary™ hack because generics
			return fmt.Sprintf("*OSource_Phrase[%s]", ocaml2go(asCons[:len(asCons)-1], currentModule))
		}
		if lastAsPrimitive, ok := last.(ocaml.Primitive); ok {
			switch lastAsPrimitive {
			case "list":
				return fmt.Sprintf("[]%s", ocaml2go(asCons[:len(asCons)-1], currentModule))
			case "option":
				return fmt.Sprintf("*%s", ocaml2go(asCons[:len(asCons)-1], currentModule))
			}
		}
		if len(asCons) == 1 {
			return ocaml2go(asCons[0], currentModule)
		}
	} else if asRecord, ok := t.(ocaml.Record); ok {
		res := "struct{"
		for _, f := range asRecord {
			res += fmt.Sprintf("%s %s; ", fieldName(f.Name), ocaml2go(f.Type, currentModule))
		}
		res += "}"
		return res
	} else if asFunc, ok := t.(ocaml.Func); ok {
		return fmt.Sprintf("func(%s) %s", ocaml2go(asFunc.In, currentModule), ocaml2go(asFunc.Out, currentModule))
	} else if asTuple, ok := t.(ocaml.Tuple); ok {
		res := "struct{"
		for i, t := range asTuple {
			res += fmt.Sprintf("F%d %s; ", i, ocaml2go(t, currentModule))
		}
		res += "}"
		return res
	}

	return fmt.Sprintf("TODO /* %s (kind %d) */", t, t.Kind())
}

var opNames = map[string]string{
	"@@": "AtAt",
	"+":  "Plus",
	"-":  "Minus",
	"=":  "Eq",
	"<>": "NotEq",
	">":  "Gt",
	"<":  "Lt",
	">=": "Gte",
	"<=": "Lte",
	"||": "Or",
}

func init() {
	lang := tree_sitter.NewLanguage(tree_sitter_ocaml.LanguageOCaml())
	ocamlParser = tree_sitter.NewParser()
	ocamlParser.SetLanguage(lang)
}

type ocamlParse struct {
	filepath string
	source   []byte
	tree     *tree_sitter.Tree
}

func newOcamlParse(path string) ocamlParse {
	lspClient.DidOpen(path)
	source := utils.Must1(os.ReadFile(path))
	tree := ocamlParser.Parse(source, nil)
	return ocamlParse{
		filepath: utils.Must1(filepath.Abs(path)),
		source:   source,
		tree:     tree,
	}
}

func main() {
	rootCmd := &cobra.Command{
		Use: "gen",
		Run: func(cmd *cobra.Command, args []string) {
			lspClient = ocaml.NewOCamlClient(filepath.Join(specpath, "interpreter"))
			defer lspClient.Close()

			outFile = utils.Must1(os.Create("generated.go"))
			defer outFile.Close()
			defer outFile.Sync()

			w("// This file is automatically generated. DO NOT EDIT.\n")
			w("package parser\n\n")

			for _, f := range files {
				p := newOcamlParse(filepath.Join(append([]string{specpath}, f.Path...)...))
				mod := ocaml.NewModule(f.ModuleName)
				modules[mod.Name] = mod

				root := p.tree.RootNode()
				for _, child := range root.NamedChildren(root.Walk()) {
					switch child.GrammarName() {
					case "type_definition":
						if f.SkipTypes {
							continue
						}
						defs := p.parseTypeDef(&child, f, mod)
						for _, def := range defs {
							mod.Defs[def.Name] = def
						}
					case "value_definition":
						for _, def := range child.NamedChildren(child.Walk()) {
							switch def.GrammarName() {
							case "let_binding":
								pattern := def.ChildByFieldName("pattern")
								t := p.getTypeStart(pattern, mod)

								if slices.Contains(f.Skip, p.s(pattern)) {
									fmt.Fprintf(os.Stderr, "skipping %s = ...\n", p.s(pattern))
									continue
								}

								switch t.(type) {
								case ocaml.Func:
									if f.AllFuncs || slices.Contains(toTranslate, p.s(pattern)) {
										if !f.SkipFuncs {
											p.parseFunc(&def, mod)
										}
									}
								case ocaml.TypeDef:
									p.parseValueDef(&def, mod)
								default:
									w("// TODO: Unknown type for definition of %s: %s\n\n", p.s(pattern), t)
								}
							}
						}
					case "open_module":
						modName := p.s(child.NamedChild(0))
						if otherMod, ok := modules[modName]; ok {
							for name, def := range otherMod.Defs {
								if _, existing := mod.Defs[name]; existing {
									fmt.Fprintf(os.Stderr, "WARNING: %s.%s overrides existing definition for %s in module %s\n", otherMod.Name, name, name, mod.Name)
								}
								mod.Defs[name] = def
							}
						} else {
							fmt.Fprintf(os.Stderr, "WARNING: in module %s: no module defined with name %s, so inheriting no definitions\n", mod.Name, modName)
						}
					case "module_definition":
						p.parseModuleDef(&child, f, mod)
					case "comment":
					default:
						fmt.Fprintf(os.Stderr, "skipping unknown %s\n", child.GrammarName())
					}
				}
			}

			writeUnpacks()
		},
	}

	utils.Must(rootCmd.Execute())
}

func (p *ocamlParse) s(n *tree_sitter.Node) string {
	return n.Utf8Text(p.source)
}

func (p *ocamlParse) parseTypeDef(n *tree_sitter.Node, f File, currentModule *ocaml.Module) []ocaml.TypeDef {
	var defs []ocaml.TypeDef
	for _, binding := range n.NamedChildren(n.Walk()) {
		if binding.GrammarName() != "type_binding" {
			fmt.Fprintf(os.Stderr, "spurious %s while processing type definitions\n", binding.GrammarName())
			continue
		}

		nName := binding.ChildByFieldName("name")
		nBody := binding.NamedChild(1)

		name := p.s(nName)

		if slices.Contains(f.Skip, name) {
			fmt.Fprintf(os.Stderr, "skipping type %s = ...\n", name)
			continue
		}

		fmt.Fprintf(os.Stderr, "parsing type %s = ...\n", name)

		// fmt.Fprintf(os.Stderr, "parsing type %s: %s\n", name, p.s(n))
		// fmt.Fprintf(os.Stderr, "  %s\n", nBody.ToSexp())
		if existingType, ok := currentModule.Defs[name]; ok {
			fmt.Fprintf(os.Stderr, "WARNING: duplicate definition of type %s: already had %s but got %s as well\n", p.s(nName), existingType, p.s(nBody))
		}

		def := ocaml.TypeDef{
			Identifier: ocaml.Identifier{
				Modules: currentModule.Namespace(),
				Name:    name,
			},
			Type: p.parseTypeDecl(nBody, currentModule),
		}
		defs = append(defs, def)
		p.writeTypeDef(def, currentModule)
	}
	return defs
}

func (p *ocamlParse) parseTypeDecl(n *tree_sitter.Node, currentModule *ocaml.Module) ocaml.Type {
	// fmt.Fprintf(os.Stderr, "type decl %s: %s\n", n.GrammarName(), p.s(n))
	// fmt.Fprintf(os.Stderr, "  %s\n", n.ToSexp())

	name := p.s(n)
	if existingType, ok := currentModule.Defs[name]; ok {
		return existingType
	}

	switch n.GrammarName() {
	case "_lowercase_identifier":
		return ocaml.Identifier{currentModule.Namespace(), name}
	case "type_constructor_path":
		ty := p.parseTypeDecl(n.NamedChild(n.NamedChildCount()-1), currentModule)
		switch ty := ty.(type) {
		case ocaml.Identifier:
			var modules []string
			for i := range n.NamedChildCount() - 1 {
				modules = append(modules, p.s(n.NamedChild(i)))
			}
			if len(modules) > 0 {
				ty.Modules = modules
			}
			return ty
		case ocaml.TypeDef:
			return ty
		default:
			exitWithError("how can you even have a %+v in a type_constructor_path", ty)
			return nil
		}
	case "constructed_type":
		var cons ocaml.Cons
		tl := p.parseTypeDecl(n.NamedChild(0), currentModule)
		tr := p.parseTypeDecl(n.NamedChild(1), currentModule)
		if tlcons, ok := tl.(ocaml.Cons); ok {
			cons = append(cons, tlcons...)
		} else {
			cons = append(cons, tl)
		}
		cons = append(cons, tr)
		return cons
	case "function_type":
		in := p.parseTypeDecl(n.NamedChild(0), currentModule)
		out := p.parseTypeDecl(n.NamedChild(1), currentModule)
		return ocaml.Func{
			In:  in,
			Out: out,
		}
	case "variant_declaration":
		var variants ocaml.Variants
		for i := range n.NamedChildCount() {
			if n.NamedChild(i).GrammarName() == "comment" {
				continue
			}
			constructor := Lookup{n}.Child(i, "constructor_declaration").Node
			constructorName := constructor.NamedChild(0)
			variant := ocaml.Variant{
				Name: p.s(constructorName),
			}
			if constructor.NamedChildCount() > 1 {
				var tup ocaml.Tuple
				for i := uint(1); i < constructor.NamedChildCount(); i++ {
					tup = append(tup, p.parseTypeDecl(constructor.NamedChild(i), currentModule))
				}
				if len(tup) == 1 {
					variant.Type = &tup[0]
				} else {
					var t ocaml.Type = tup
					variant.Type = &t
				}
			}
			variants = append(variants, variant)
		}
		return variants
	case "tuple_type":
		var tup ocaml.Tuple
		tl := p.parseTypeDecl(n.NamedChild(0), currentModule)
		tr := p.parseTypeDecl(n.NamedChild(1), currentModule)
		if tltup, ok := tl.(ocaml.Tuple); ok {
			tup = append(tup, tltup...)
		} else {
			tup = append(tup, tl)
		}
		tup = append(tup, tr)
		return tup
	case "record_declaration":
		var rec ocaml.Record
		for _, f := range n.NamedChildren(n.Walk()) {
			if f.GrammarName() != "field_declaration" {
				continue
			}
			name := f.NamedChild(0)
			t := p.parseTypeDecl(f.NamedChild(1), currentModule)
			rec = append(rec, ocaml.RecordField{
				Name: p.s(name),
				Type: t,
			})
		}
		return rec
	case "type_variable":
		return ocaml.Identifier{nil, "UnknownTypeVariable_" + varName(nil, name)}
	default:
		exitWithError("unexpected type declaration node %s", n.GrammarName())
		return nil
	}
}

func (p *ocamlParse) writeTypeDef(def ocaml.TypeDef, currentModule *ocaml.Module) {
	switch t := def.Type.(type) {
	case ocaml.Identifier, ocaml.Cons, ocaml.Func, ocaml.Primitive:
		w("type %s = %s\n", typeName(def.Modules, def.Name), ocaml2go(t, currentModule))
	case ocaml.Tuple:
		w("type %s = struct {\n", typeName(def.Modules, def.Name))
		for i, f := range t {
			w("  F%d %s\n", i, ocaml2go(f, currentModule))
		}
		w("}\n")
	case ocaml.Variants:
		tn := typeName(def.Modules, def.Name)
		kindName := typeName(def.Modules, def.Name+"_kind")
		w("\ntype %s int\n\n", kindName)

		w("const(\n")
		for i, variant := range t {
			w("%s", variantKindName(def.Modules, variant.Name))
			if i == 0 {
				w(" %s = iota + 1", kindName)
			}
			w("\n")
		}
		w(")\n\n")

		w("type %s interface {\n", tn)
		w("  Kind() %s\n", kindName)
		w("}\n\n")

		w("type Simple%s struct {\n", tn)
		w("  kind %s\n", kindName)
		w("}\n\n")

		w("func (t Simple%s) Kind() %s {\n", tn, kindName)
		w("  return t.kind\n")
		w("}\n\n")

		for _, variant := range t {
			if variant.Type == nil {
				w("var %s %s = Simple%s{%s}\n", varName(def.Modules, variant.Name), tn, tn, variantKindName(def.Modules, variant.Name))
			} else {
				variantTypeName := tn + "_" + variant.Name
				w("type %s struct {\n", variantTypeName)
				w("  V %s\n", ocaml2go(*variant.Type, currentModule))
				w("}\n")

				w("func (t %s) Kind() %s {\n", variantTypeName, kindName)
				w("  return %s\n", variantKindName(def.Modules, variant.Name))
				w("}\n")

				w("func %s(v %s) %s {\n", funcName(def.Modules, variant.Name, 1), ocaml2go(*variant.Type, currentModule), tn)
				w("  return %s{v}\n", variantTypeName)
				w("}\n")
			}
		}
	case ocaml.Record:
		w("type %s struct {\n", typeName(def.Modules, def.Name))
		for _, f := range t {
			w("  %s %s\n", fieldName(f.Name), ocaml2go(f.Type, currentModule))
		}
		w("}\n")
	default:
		exitWithError("don't know how to write type %s = %s", def.Name, def.Type)
	}
}

func (p *ocamlParse) parseFunc(f *tree_sitter.Node, currentModule *ocaml.Module) {
	utils.Assert(f.GrammarName() == "let_binding", "expected a let")

	// fmt.Fprintf(os.Stderr, "func? %s\n", f.ToSexp())

	pattern := Lookup{f}.Field("pattern", "").Node
	body := Lookup{f}.Field("body", "").Node

	fmt.Fprintf(os.Stderr, "parsing func %s = ...\n", p.s(pattern))

	var params []*tree_sitter.Node
	for _, child := range f.NamedChildren(f.Walk()) {
		if child.GrammarName() == "parameter" {
			params = append(params, &child)
		}
	}

	tmpCount = 0

	name := p.s(pattern)
	funcType := p.getTypeStart(pattern, currentModule).(ocaml.Func)
	funcResultType := funcType.GetTypeAfterApplyingArgs(len(params))

	fullFuncName := funcName(currentModule.Namespace(), name, len(params))
	w("func %s(", fullFuncName)
	for _, param := range params {
		paramName := varName(nil, p.s(param))
		paramType := p.getTypeEnd(param, currentModule)
		w("%s %s, ", paramName, ocaml2go(paramType, currentModule))
	}
	w(") %s {\n", ocaml2go(funcResultType, currentModule))
	p.parseExpr(body, funcResultType, currentModule, true, true)
	w("}\n\n")

	for i := len(params) - 1; i >= 1; i-- {
		w("func %s(", funcName(currentModule.Namespace(), name, i))
		for j := 0; j < i; j++ {
			param := params[j]
			paramName := varName(nil, p.s(param))
			paramType := p.getTypeEnd(param, currentModule)
			w("%s %s, ", paramName, ocaml2go(paramType, currentModule))
		}
		w(") func(")
		for j := i; j < len(params); j++ {
			param := params[j]
			paramName := varName(nil, p.s(param))
			paramType := p.getTypeEnd(param, currentModule)
			w("%s %s, ", paramName, ocaml2go(paramType, currentModule))
		}
		w(") %s {\n", ocaml2go(funcResultType, currentModule))
		w("  return func(")
		for j := i; j < len(params); j++ {
			param := params[j]
			paramName := varName(nil, p.s(param))
			paramType := p.getTypeEnd(param, currentModule)
			w("%s %s, ", paramName, ocaml2go(paramType, currentModule))
		}
		w(") %s {\n", ocaml2go(funcResultType, currentModule))
		w("    return %s(", fullFuncName)
		for _, param := range params {
			w("%s, ", varName(nil, p.s(param)))
		}
		w(")\n")
		w("  }\n")
		w("}\n\n")
	}

	baseName := funcName(currentModule.Namespace(), p.s(pattern), -1)
	w("var %s = %s\n\n", baseName, fullFuncName)
}

func (p *ocamlParse) parseValueDef(def *tree_sitter.Node, currentModule *ocaml.Module) {
	pattern := def.ChildByFieldName("pattern")
	body := def.ChildByFieldName("body")

	fmt.Fprintf(os.Stderr, "parsing value %s = ...\n", p.s(pattern))

	expectedType := p.getTypeStart(pattern, currentModule)

	w("var %s = ", varName(currentModule.Namespace(), p.s(pattern)))
	p.parseExpr(body, expectedType, currentModule, false, false)
	w("\n")
}

func (p *ocamlParse) parseModuleDef(def *tree_sitter.Node, f File, currentModule *ocaml.Module) {
	binding := def.NamedChild(0)
	name := binding.ChildByFieldName("name")
	body := binding.ChildByFieldName("body")

	switch body.GrammarName() {
	case "structure":
		newMod := ocaml.NewModule(p.s(name))
		newMod.ParentModules = currentModule.Namespace()

		for _, def := range body.NamedChildren(body.Walk()) {
			switch def.GrammarName() {
			case "type_definition":
				// We need to parse the thing in the current context, including opened defs
				// from the outer module, but we also need functions to be generated with the
				// right name. So we cheat.
				phonyModule := *newMod
				phonyModule.Defs = currentModule.Defs
				defs := p.parseTypeDef(&def, f, &phonyModule)
				for _, def := range defs {
					newMod.Defs[def.Name] = def
				}
			default:
				w("// Ignoring %s in module definition\n", def.GrammarName())
			}
		}

		modules[newMod.Name] = newMod
	case "module_path":
		// Module alias
		thisName := p.s(name)
		otherName := p.s(body)
		modules[thisName] = modules[otherName]
	case "module_application":
		// Ignore
	default:
		exitWithError("Unknown type of body for module definition: %s", body.GrammarName())
	}
}

var reUnsafeChar = regexp.MustCompile("[^a-zA-Z0-9_]")

func snake2camel(s string) string {
	parts := strings.Split(s, "_")
	for i := range parts {
		if parts[i] != "" {
			parts[i] = strings.ToUpper(parts[i][:1]) + parts[i][1:]
		}
	}
	return strings.Join(parts, "")
}

func camelName(modulePath []string, name string) string {
	var parts []string
	for _, n := range modulePath {
		parts = append(parts, reUnsafeChar.ReplaceAllString(snake2camel(n), "_"))
	}
	parts = append(parts, reUnsafeChar.ReplaceAllString(name, "_"))
	return strings.Join(parts, "_")
}

func varName(modulePath []string, name string) string {
	res := ""
	if len(modulePath) > 0 {
		var parts []string
		for _, n := range modulePath {
			parts = append(parts, reUnsafeChar.ReplaceAllString(snake2camel(n), "_"))
		}
		res += strings.Join(parts, "_")
	}
	res += "_" + reUnsafeChar.ReplaceAllString(name, "_")
	return res
}

func funcName(modulePath []string, name string, numArgs int) string {
	res := camelName(modulePath, name)
	if numArgs >= 0 {
		res += fmt.Sprintf("_%d", numArgs)
	}
	return res
}

func typeName(modulePath []string, name string) string {
	return "O" + camelName(modulePath, name)
}

func variantKindName(modulePath []string, name string) string {
	return "K" + camelName(modulePath, name)
}

func fieldName(name string) string {
	return camelName(nil, name)
}

func (p *ocamlParse) parseExpr(
	expr *tree_sitter.Node,
	expectedType ocaml.Type,
	module *ocaml.Module,
	statement bool,
	returnIfTerminal bool,
) string {
	// fmt.Fprintf(os.Stderr, "parsing %s (expecting: %s, in module: %s, as statement: %v, returning if terminal: %v)\n", expr.GrammarName(), expectedType, currentModule, statement, returnIfTerminal)
	// fmt.Fprintf(os.Stderr, "  %s\n", p.s(expr))
	// fmt.Fprintf(os.Stderr, "  %s\n", expr.ToSexp())

	utils.Assert(module != nil, "must have a module to parse expressions")

	if returnIfTerminal && !statement {
		exitWithError("for %s expression: cannot return a non-statement", expr.GrammarName())
	}

	switch expr.GrammarName() {
	case "value_path", "_lowercase_identifier", "_uppercase_identifier":
		res := tmpVar()
		if statement {
			w("%s := ", res)
		}
		name := varName(nil, p.s(expr))
		w("%s", name)
		if statement {
			w("\n")
			if returnIfTerminal {
				w("return %s\n", res)
				return ""
			}
			return res
		}
	case "constructor_path":
		res := tmpVar()
		if statement {
			w("%s := ", res)
		}
		for i := range expr.NamedChildCount() {
			if i < expr.NamedChildCount()-1 {
				w("/*%s.*/", p.s(expr.NamedChild(i)))
			} else {
				p.parseExpr(expr.NamedChild(i), expectedType, module, false, false)
			}
		}
		if statement {
			w("\n")
			if returnIfTerminal {
				w("return %s\n", res)
				return ""
			}
			return res
		}
	case "number", "signed_number":
		n := p.s(expr)
		n = strings.TrimRight(n, "lL")
		w("%s", n)
	case "or_pattern", "tuple_pattern":
		p.parseExpr(expr.NamedChild(0), nil, module, false, false)
		w(", ")
		p.parseExpr(expr.NamedChild(1), nil, module, false, false)
	case "add_operator", "mult_operator", "pow_operator", "rel_operator", "concat_operator":
		// TODO: Implement more of these:
		// https://ocaml.org/manual/5.3/expr.html
		switch p.s(expr) {
		case "=":
			w("==")
		case "<>":
			w("!=")
		case "land":
			w("&")
		default:
			w(" %s ", p.s(expr))
		}
	case "string":
		w("%s", p.s(expr))

	case "application_expression":
		function := expr.ChildByFieldName("function")
		args := expr.ChildrenByFieldName("argument", expr.Walk())

		var funcType ocaml.Func
		if function.GrammarName() == "parenthesized_expression" {
			// HACK: A parenthesized expression as a function is spooky, but we can cheat by hovering carefully over the contents.
			funcType = p.getTypeEnd(function.NamedChild(0), module).(ocaml.Func)
		} else {
			funcType = p.getTypeEnd(function, module).(ocaml.Func)
		}

		res := tmpVar()
		if statement {
			w("%s := ", res)
		}

		w("%s(", funcName(module.Namespace(), p.s(function), len(args)))
		for i, arg := range args {
			p.parseExpr(&arg, funcType.GetArgType(i), module, false, false)
			w(", ")
		}
		w(")")

		if statement {
			w("\n")
		}
		if returnIfTerminal {
			w("return %s\n", res)
			return ""
		} else if statement {
			return res
		} else {
			return ""
		}
	case "field_get_expression":
		w("nil /* TODO: field_get_expression */")
	case "fun_expression":
		body := expr.ChildByFieldName("body")
		var params []*tree_sitter.Node
		for i := range expr.NamedChildCount() {
			child := expr.NamedChild(i)
			if child.Id() == body.Id() {
				break
			}
			params = append(params, child)
		}

		funcType := p.getTypeStart(expr, module).(ocaml.Func)

		w("func(")
		for _, param := range params {
			paramName := varName(nil, p.s(param))
			paramType := p.getTypeEnd(param, module)
			w("%s %s, ", paramName, ocaml2go(paramType, module))
		}
		w(") %s {\n", ocaml2go(funcType.Out, module))

		p.parseExpr(body, funcType.Out, module, true, true)

		w("}")
	case "if_expression":
		condition := expr.ChildByFieldName("condition")

		res := tmpVar()

		if !statement {
			// Emit an inline, immediately-invoked function
			w("func() %s {\n", ocaml2go(expectedType, module))
		}

		w("var %s %s\n", res, ocaml2go(expectedType, module))

		w("if ")
		p.parseExpr(condition, ocaml.Identifier{nil, "bool"}, module, false, false)
		for _, child := range expr.NamedChildren(expr.Walk()) {
			if child.Id() == condition.Id() {
				continue
			}

			switch child.GrammarName() {
			case "then_clause":
				w(" {\n")
				thenRes := p.parseExpr(child.NamedChild(0), expectedType, module, true, false)
				if len(res) > 0 {
					w("%s = %s\n", res, thenRes)
				}
				w("} ")
			case "else_clause":
				w(" else {\n")
				elseRes := p.parseExpr(child.NamedChild(0), expectedType, module, true, false)
				if len(res) > 0 {
					w("%s = %s\n", res, elseRes)
				}
				w("} ")
			default:
				exitWithError("unknown type in if expression: %s", child.GrammarName())
			}
		}
		w("\n")

		if !statement {
			w("return %s\n", res)
			w("}()")
			return ""
		} else if returnIfTerminal {
			w("return %s\n", res)
			return ""
		} else {
			return res
		}
	case "infix_expression":
		left := expr.ChildByFieldName("left")
		operator := expr.ChildByFieldName("operator")
		right := expr.ChildByFieldName("right")

		res := tmpVar()
		if statement {
			w("%s := ", res)
		}

		opType := p.getTypeEnd(operator, module).(ocaml.Func)
		infixOpGoName, ok := opNames[p.s(operator)]
		if !ok {
			infixOpGoName = p.s(operator)
		}

		funcName := fmt.Sprintf("_operator%s_2", infixOpGoName)
		// if opType.GetArgType(0).String()[0] != '\'' {
		// 	funcName = fmt.Sprintf("_%s", opType.GetArgType(0)) + funcName
		// }

		w("%s(", funcName)
		p.parseExpr(left, opType.GetArgType(0), module, false, false)
		w(", ")
		p.parseExpr(right, opType.GetArgType(1), module, false, false)
		w(")")

		if statement {
			w("\n")
		}

		if returnIfTerminal {
			w("return %s\n", res)
			return ""
		} else if statement {
			return res
		} else {
			return ""
		}
	case "let_expression":
		if !statement {
			exitWithError("cannot use let_expression as an expression")
		}

		binding := Lookup{expr}.
			Child(0, "value_definition").
			Child(0, "let_binding").
			Node
		pattern := Lookup{binding}.Field("pattern", "").Node
		body := Lookup{binding}.Field("body", "").Node

		var bindingType ocaml.Type
		if pattern.GrammarName() == "tuple_pattern" {
			var tup ocaml.Tuple
			for _, v := range flattenTuplePattern(pattern) {
				tup = append(tup, p.getTypeEnd(v, module))
			}
			bindingType = tup
		} else {
			bindingType = p.getTypeEnd(pattern, module)
		}
		bindingRes := p.parseExpr(body, bindingType, module, true, false)

		p.parseExpr(pattern, nil, module, false, false)
		w(" := ")
		if pattern.GrammarName() == "tuple_pattern" {
			unpackName := trackUnpack(bindingType.(ocaml.Tuple), module)
			w("%s(%s)", unpackName, bindingRes)
		} else {
			w("%s", bindingRes)
		}
		w("\n")

		return p.parseExpr(expr.NamedChild(1), expectedType, module, true, returnIfTerminal)
	case "list_expression":
		listType := p.getTypeEnd(expr, module)
		var elemType ocaml.Type

		if asCons, ok := listType.(ocaml.Cons); ok {
			// if len(asCons) != 2 || asCons[1] != ocaml.NamedType{nil,"list"} {
			// 	exitWithError("list_expression needs a list type, but got: %s", expectedType)
			// }
			elemType = asCons[0]
		} else {
			exitWithError("list_expression needs a cons type (that is a list), but got: %s", expectedType)
		}

		// TODO: Statement mode

		w("[]%s{", ocaml2go(elemType, module))
		for _, child := range expr.NamedChildren(expr.Walk()) {
			p.parseExpr(&child, elemType, module, false, false)
			w(", ")
		}
		w("}")
	case "local_open_expression":
		// e.g. "Int32.(add lo (shift_left hi 16))"
		modName := p.s(expr.NamedChild(0))
		localMod := modules[modName]
		if localMod == nil {
			localMod = ocaml.NewModule(modName)
		}
		return p.parseExpr(expr.NamedChild(1), expectedType, localMod, statement, returnIfTerminal)
	case "match_expression":
		matchResult := tmpVar()
		w("var %s %s\n", matchResult, ocaml2go(expectedType, module))

		matchVar := tmpVar()
		w("%s := ", matchVar)
		p.parseExpr(expr.NamedChild(0), nil, module, false, false)
		w("\n")

		for i, matchCase := range expr.NamedChildren(expr.Walk())[1:] {
			if matchCase.GrammarName() != "match_case" {
				continue
			}

			pattern := Lookup{&matchCase}.Field("pattern", "").Node
			body := Lookup{&matchCase}.Field("body", "").Node
			var guard *tree_sitter.Node
			for _, child := range matchCase.NamedChildren(matchCase.Walk()) {
				if child.GrammarName() == "guard" {
					guard = child.NamedChild(0)
				}
			}

			if i == 0 {
				w("if ")
			} else {
				w("} else if ")
			}

			// Will open the body of the if
			p.parseMatchPattern(pattern, matchVar, guard, module)

			res := p.parseExpr(body, expectedType, module, true, false)
			if len(matchResult) > 0 {
				w("%s = %s", matchResult, res)
			}

			w("\n")
		}

		w("}\n")

		if returnIfTerminal {
			w("return %s\n", matchResult)
			return ""
		} else {
			return matchResult
		}
	case "parenthesized_expression":
		return p.parseExpr(expr.NamedChild(0), expectedType, module, statement, returnIfTerminal)
	case "product_expression":
		nodes := flattenProductExpression(expr)

		res := tmpVar()
		if returnIfTerminal {
			w("return ")
		} else if statement {
			w("%s := ", res)
		}

		var tup ocaml.Tuple
		switch t := expectedType.(type) {
		case ocaml.Tuple:
			tup = t
		case ocaml.TypeDef:
			tup = t.Type.(ocaml.Tuple)
		default:
			exitWithError("unexpected type in product_expression: %s (kind %d)", expectedType, expectedType.Kind())
		}

		utils.Assert(len(nodes) == len(tup), "mismatch between product values and expected tuple type")

		w("%s{", ocaml2go(tup, module))
		for i, n := range nodes {
			p.parseExpr(n, tup[i], module, false, false)
			w(", ")
		}
		w("}")

		if statement {
			w("\n")
			if returnIfTerminal {
				return ""
			} else {
				return res
			}
		}
	case "record_expression":
		w("nil /* TODO: record_expression */")
	case "sequence_expression":
		if !statement {
			exitWithError("cannot use sequence_expression as an expression")
		}

		left := expr.ChildByFieldName("left")
		right := expr.ChildByFieldName("right")

		leftRes := p.parseExpr(left, nil, module, true, false)
		if leftRes != "" {
			w("_ = %s\n", leftRes)
		}

		rightRes := p.parseExpr(right, expectedType, module, true, returnIfTerminal)
		w("\n")

		return rightRes
	case "sign_expression":
		w("%s(", p.s(expr.ChildByFieldName("operator")))
		p.parseExpr(expr.ChildByFieldName("right"), expectedType, module, false, false)
		w(")")
	default:
		w("TODO /* unknown expression type %s */", expr.GrammarName())
	}

	return ""
}

// You are expected to write the start of the if case before calling this,
// e.g. "if " or "} else if ".
func (p *ocamlParse) parseMatchPattern(
	pattern *tree_sitter.Node,
	matchVar string,
	guard *tree_sitter.Node,
	currentModule *ocaml.Module,
) {
	utils.Assert(currentModule != nil, "must have a module to parse match patterns")

	switch pattern.GrammarName() {
	case "_lowercase_identifier":
		p.parseExpr(pattern, nil, currentModule, false, false)
		w(" := %s; ", matchVar)
		if guard == nil {
			w("true")
		} else {
			p.parseExpr(guard, ocaml.Identifier{nil, "bool"}, currentModule, false, false)
		}
		w(" {\n")

		// Ignore in case it is unused
		w("_ = ")
		p.parseExpr(pattern, nil, currentModule, false, false)
		w("\n")
	case "number", "signed_number":
		w("%s == ", matchVar)
		p.parseExpr(pattern, nil, currentModule, false, false)
		utils.Assert(guard == nil, "expected no guard")
		w(" {\n")
	case "alias_pattern":
		p.parseMatchPattern(pattern.NamedChild(0), matchVar, nil, currentModule)
		p.parseExpr(pattern.NamedChild(1), nil, currentModule, false, false)
		w(" := %s\n", matchVar)
		utils.Assert(guard == nil, "expected no guard")
	case "constructor_pattern":
		// We only handle Some and None.
		switch p.s(pattern.NamedChild(0)) {
		case "Some":
			p.parseExpr(pattern.NamedChild(1), nil, currentModule, false, false)
			w(" := __derefIfNotNil(%s); %s != nil ", matchVar, matchVar)
			if guard != nil {
				w("&& (")
				p.parseExpr(guard, ocaml.Identifier{nil, "bool"}, currentModule, false, false)
				w(") ")
			}
			w("{\n")
		case "None":
			w("%s == nil {\n", matchVar)
		default:
			exitWithError("unknown constructor in match case: %s", pattern.GrammarName())
		}
	case "or_pattern":
		for i, orValue := range flattenOrPattern(pattern) {
			if i > 0 {
				w("||")
			}
			w("%s == ", matchVar)
			p.parseExpr(orValue, nil, currentModule, false, false)
		}
		w(" {\n")
		utils.Assert(guard == nil, "expected no guard")
	default:
		exitWithError("unknown type of match case: %s", pattern.GrammarName())
	}
}

func flattenTuplePattern(p *tree_sitter.Node) []*tree_sitter.Node {
	switch p.GrammarName() {
	case "tuple_pattern":
		var res []*tree_sitter.Node
		res = append(res, flattenTuplePattern(p.NamedChild(0))...)
		res = append(res, flattenTuplePattern(p.NamedChild(1))...)
		return res
	default:
		return []*tree_sitter.Node{p}
	}
}

func flattenOrPattern(p *tree_sitter.Node) []*tree_sitter.Node {
	switch p.GrammarName() {
	case "or_pattern":
		var res []*tree_sitter.Node
		res = append(res, flattenOrPattern(p.NamedChild(0))...)
		res = append(res, flattenOrPattern(p.NamedChild(1))...)
		return res
	case "parenthesized_pattern":
		return flattenOrPattern(p.NamedChild(0))
	default:
		return []*tree_sitter.Node{p}
	}
}

func flattenProductExpression(p *tree_sitter.Node) []*tree_sitter.Node {
	switch p.GrammarName() {
	case "product_expression":
		var res []*tree_sitter.Node
		res = append(res, flattenProductExpression(p.NamedChild(0))...)
		res = append(res, flattenProductExpression(p.NamedChild(1))...)
		return res
	default:
		return []*tree_sitter.Node{p}
	}
}

func w(msg string, args ...any) {
	fmt.Fprintf(outFile, msg, args...)
}

func tmpVar() string {
	tmpCount += 1
	return fmt.Sprintf("__tmp%d", tmpCount)
}

func parseHoverResponse(hover ocaml.M, currentModule *ocaml.Module) ocaml.Type {
	value := hover["contents"].(ocaml.M)["value"].(string)
	value = strings.SplitN(value, "***", 2)[0]
	return ocaml.ParseType(value, currentModule)
}

func (p *ocamlParse) getTypeStart(node *tree_sitter.Node, currentModule *ocaml.Module) ocaml.Type {
	hover := utils.Must1(lspClient.Hover(
		p.filepath,
		int(node.StartPosition().Row),
		int(node.StartPosition().Column),
	))
	return parseHoverResponse(hover, currentModule)
}

func (p *ocamlParse) getTypeEnd(node *tree_sitter.Node, currentModule *ocaml.Module) ocaml.Type {
	hover := utils.Must1(lspClient.Hover(
		p.filepath,
		int(node.EndPosition().Row),
		int(node.EndPosition().Column),
	))
	return parseHoverResponse(hover, currentModule)
}

func exitWithError(msg string, args ...any) {
	msg = fmt.Sprintf(msg, args...)
	panic(fmt.Sprintf("ERROR: %s\n", msg))
}

type Lookup struct {
	Node *tree_sitter.Node
}

func (l Lookup) Child(i uint, grammarName string) Lookup {
	node := l.Node.NamedChild(i)
	if grammarName != "" {
		utils.Assert(node.GrammarName() == grammarName, "expected %s but got %s", grammarName, node.GrammarName())
	}
	return Lookup{node}
}

func (l Lookup) Field(fieldName string, grammarName string) Lookup {
	node := l.Node.ChildByFieldName(fieldName)
	if grammarName != "" {
		utils.Assert(node.GrammarName() == grammarName, "expected %s but got %s", grammarName, node.GrammarName())
	}
	return Lookup{node}
}

var unpacks []Unpack

type Unpack struct {
	Module *ocaml.Module
	Name   string
	Type   ocaml.Tuple
}

func trackUnpack(tup ocaml.Tuple, currentModule *ocaml.Module) string {
	name := "__unpack" + varName(nil, tup.String())
	already := false
	for _, unpack := range unpacks {
		if unpack.Name == name {
			already = true
		}
	}
	if !already {
		unpacks = append(unpacks, Unpack{
			Module: currentModule,
			Name:   name,
			Type:   tup,
		})
	}
	return name
}

func writeUnpacks() {
	for _, unpack := range unpacks {
		w("func %s(t %s) (", unpack.Name, ocaml2go(unpack.Type, unpack.Module))
		for _, t := range unpack.Type {
			w("%s, ", ocaml2go(t, unpack.Module))
		}
		w(") {\n")
		w("  return ")
		for i := range unpack.Type {
			if i > 0 {
				w(", ")
			}
			w("t.F%d", i)
		}
		w("\n")
		w("}\n\n")
	}
}
