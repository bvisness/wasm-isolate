package main

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/bvisness/wasm-isolate/utils"
	"github.com/spf13/cobra"
	tree_sitter "github.com/tree-sitter/go-tree-sitter"
	tree_sitter_ocaml "github.com/tree-sitter/tree-sitter-ocaml/bindings/go"
)

var specpath = filepath.Join("gen", "spec")

var source []byte
var outFile *os.File

func main() {
	var rootCmd *cobra.Command
	rootCmd = &cobra.Command{
		Use: "gen",
		Run: func(cmd *cobra.Command, args []string) {
			source = utils.Must1(os.ReadFile(filepath.Join(specpath, "interpreter", "binary", "decode.ml")))
			outFile = utils.Must1(os.Create("generated.go"))
			defer outFile.Close()

			w("// This file is automatically generated. DO NOT EDIT.\n")
			w("package parser\n\n")

			ocaml := tree_sitter.NewLanguage(tree_sitter_ocaml.LanguageOCaml())
			parser := tree_sitter.NewParser()
			defer parser.Close()
			parser.SetLanguage(ocaml)

			tree := parser.Parse(source, nil)
			defer tree.Close()

			root := tree.RootNode()
			cur := root.Walk()

			// Find `let rec instr s = ...`, the instruction-parsing function.
			var instr *tree_sitter.Node
			for _, child := range root.NamedChildren(cur) {
				if child.GrammarName() != "value_definition" {
					continue
				}
				binding := child.NamedChild(0)
				if binding.GrammarName() != "let_binding" {
					continue
				}
				pattern := binding.ChildByFieldName("pattern")
				if s(pattern) != "instr" {
					continue
				}

				instr = &child
				break
			}
			if instr == nil {
				exitWithError("Couldn't find `let rec instr s ...`")
			}

			parseFunc(instr)
		},
	}

	utils.Must(rootCmd.Execute())
}

func s(n *tree_sitter.Node) string {
	return n.Utf8Text(source)
}

func parseFunc(f *tree_sitter.Node) {
	utils.Assert(f.GrammarName() == "value_definition", "expected a let")
	cur := f.Walk()

	binding := Lookup{f}.Child(0, "let_binding").Node
	pattern := Lookup{binding}.Field("pattern", "").Node
	body := Lookup{binding}.Field("body", "").Node
	var params []*tree_sitter.Node
	for _, child := range binding.NamedChildren(cur) {
		fmt.Fprintf(os.Stderr, "%s\n", child.GrammarName())
		if child.GrammarName() == "parameter" {
			params = append(params, &child)
		}
	}

	name := s(pattern)
	funcTypes := types[name]

	w("func _%s(", s(pattern))
	for _, param := range params {
		paramName := s(param)
		w("%s %s, ", paramName, funcTypes[paramName])
	}
	w(") %s {\n", funcTypes[ret])

	parseExpr(body)

	w("}\n\n")
}

func parseExpr(expr *tree_sitter.Node) {
	switch expr.GrammarName() {
	case "value_path":
		w("%s", s(expr))
	case "let_expression":
		binding := Lookup{expr}.
			Child(0, "value_definition").
			Child(0, "let_binding").
			Node
		pattern := Lookup{binding}.Field("pattern", "").Node
		body := Lookup{binding}.Field("body", "").Node

		w("%s := ", s(pattern))
		parseExpr(body)
		w("\n")

		parseExpr(expr.NamedChild(1))
	case "match_expression":
		w("switch __switchVal := ")
		parseExpr(expr.NamedChild(0))
		w("; __switchVal {\n")

		for _, matchCase := range expr.NamedChildren(expr.Walk())[1:] {
			pattern := Lookup{&matchCase}.Field("pattern", "").Node
			switch pattern.GrammarName() {
			case "number":
				w("case %s:\n", s(pattern))
			case "alias_pattern":
			case "_lowercase_identifier":
			default:
				exitWithError("unknown type of match case: %s", pattern.GrammarName())
			}
		}

		w("}\n")
	case "application_expression":
		function := Lookup{expr}.Field("function", "value_path").Node
		args := expr.ChildrenByFieldName("argument", expr.Walk())
		w("_%s(", s(function))
		for _, arg := range args {
			parseExpr(&arg)
			w(", ")
		}
		w(")")
	default:
		exitWithError("unknown expression type %s", expr.GrammarName())
	}
}

func w(msg string, args ...any) {
	fmt.Fprintf(outFile, msg, args...)
}

func exitWithError(msg string, args ...any) {
	msg = fmt.Sprintf(msg, args...)
	fmt.Fprintf(os.Stderr, "ERROR: %s\n", msg)
	os.Exit(1)
}

const ret = "__ret"

var types = map[string]map[string]string{
	"instr": {
		"s": "*Stream",
		ret: "",
	},
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