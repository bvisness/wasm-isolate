package main

import (
	"fmt"
	"io"
	"os"

	"github.com/bvisness/wasm-isolate/isolate"
	"github.com/bvisness/wasm-isolate/utils"
	"github.com/spf13/cobra"
)

func main() {
	var rootCmd *cobra.Command
	rootCmd = &cobra.Command{
		Use: "wasm-isolate <file>",
		Run: func(cmd *cobra.Command, args []string) {
			if len(args) < 1 {
				rootCmd.Usage()
				os.Exit(1)
			}
			filename := args[0]

			var wasm io.Reader
			if filename == "-" {
				wasm = os.Stdin
			} else {
				var err error
				wasm, err = os.Open(filename)
				if err != nil {
					err := err.(*os.PathError)
					exitWithError("could not open file %s: %v", err.Path, err.Err)
				}
			}

			var out io.Writer
			outname := utils.Must1(rootCmd.PersistentFlags().GetString("out"))
			if outname == "-" {
				out = os.Stdout
			} else {
				var err error
				out, err = os.Create(outname)
				if err != nil {
					err := err.(*os.PathError)
					exitWithError("could not open output file %s: %v", err.Path, err.Err)
				}
			}

			err := isolate.Isolate(wasm, out, []int{})
			if err != nil {
				exitWithError("%v", err)
			}
		},
	}
	rootCmd.PersistentFlags().StringP("out", "o", "-", "The file to write output to. Defaults to stdout.")
	utils.Must(rootCmd.Execute())
}

func exitWithError(msg string, args ...any) {
	msg = fmt.Sprintf(msg, args...)
	fmt.Fprintf(os.Stderr, "ERROR: %s\n", msg)
	os.Exit(1)
}
