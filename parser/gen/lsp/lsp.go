package lsp

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"

	"github.com/bvisness/wasm-isolate/utils"
)

type Client struct {
	cmd    *exec.Cmd
	cancel func()

	r         io.ReadCloser
	w         io.WriteCloser
	requestID int
}

func NewOCamlClient(root string) *Client {
	ctx, cancel := context.WithCancel(context.Background())
	cmd := exec.CommandContext(ctx, "ocamllsp", "--stdio")
	stdin := utils.Must1(cmd.StdinPipe())
	stdout := utils.Must1(cmd.StdoutPipe())
	cmd.Stderr = os.Stderr

	utils.Must(cmd.Start())

	c := &Client{
		cmd:    cmd,
		cancel: cancel,

		r: stdout,
		w: stdin,
	}

	// Initialize
	utils.Must1(c.Initialize(root))
	c.Initialized()

	return c
}

func (c *Client) Close() {
	fmt.Fprint(os.Stderr, "shutting down the OCaml LSP\n")
	c.cancel()
	c.cmd.Wait()
}

type M = map[string]any
type A = []any

func (c *Client) Initialize(root string) (M, error) {
	return c.Request(M{
		"method": "initialize",
		"params": M{
			"rootPath": utils.Must1(filepath.Abs(root)),
			"rootUri":  "file://" + utils.Must1(filepath.Abs(root)),
			"workspaceFolders": A{
				M{
					"uri":  "file://" + utils.Must1(filepath.Abs(root)),
					"name": "root",
				},
			},

			"capabilities": M{
				"workspace": M{
					"workspaceFolders": true,
				},
				"textDocument": M{
					"synchronization": M{
						"dynamicRegistration": true,
					},
					"hover": M{
						"dynamicRegistration": true,
						"contentFormat":       A{"plaintext"},
					},
				},
			},
			"trace": "verbose",
		},
	})
}

func (c *Client) Initialized() {
	c.Notify(M{
		"method": "initialized",
		"params": M{},
	})
}

func (c *Client) DidOpen(file string) {
	c.Notify(M{
		"method": "textDocument/didOpen",
		"params": M{
			"textDocument": M{
				"uri":        "file://" + utils.Must1(filepath.Abs(file)),
				"languageId": "ocaml",
				"version":    1,
				"text":       string(utils.Must1(io.ReadAll(utils.Must1(os.Open(file))))),
			},
		},
	})
}

func (c *Client) Hover(file string, line, col int) (M, error) {
	return c.Request(M{
		"method": "textDocument/hover",
		"params": M{
			"textDocument": M{
				"uri": "file://" + utils.Must1(filepath.Abs(file)),
			},
			"position": M{
				"line":      line,
				"character": col,
			},
		},
	})
}

var reHeader = regexp.MustCompile(`^(.*?): (.*)`)

func (c *Client) Request(request M) (M, error) {
	c.requestID += 1
	request["id"] = c.requestID
	c.Send(request)
	return c.Receive(c.requestID)
}

func (c *Client) Notify(notification M) {
	c.Send(notification)
}

func (c *Client) Send(message M) {
	message["jsonrpc"] = "2.0"
	data := utils.Must1(json.Marshal(message))

	utils.Must1(c.w.Write([]byte(fmt.Sprintf("Content-Length: %d\r\n\r\n", len(data)))))
	utils.Must1(c.w.Write(data))
}

func (c *Client) Receive(id int) (M, error) {
	for {
		// Read headers
		headers := make(map[string]string)
		for {
			var rawHeader []byte

			for {
				b := utils.Must1(c.ReadByte())
				if b == '\r' {
					b2 := utils.Must1(c.ReadByte())
					if b2 == '\n' {
						break
					} else {
						panic("unexpected character after carriage return")
					}
				} else {
					rawHeader = append(rawHeader, b)
				}
			}

			if len(rawHeader) == 0 {
				break
			}

			m := reHeader.FindStringSubmatch(string(rawHeader))
			headers[m[1]] = m[2]
		}

		contentLengthStr, ok := headers["Content-Length"]
		if !ok {
			panic("missing Content-Length header")
		}
		contentLength := utils.Must1(strconv.Atoi(contentLengthStr))

		body := make([]byte, contentLength)
		utils.Must1(c.r.Read(body))

		var res M
		utils.Must(json.Unmarshal(body, &res))
		if _, ok := res["id"]; !ok {
			// Just some spurious message
			continue
		}
		if res["id"].(float64) != float64(id) {
			// Unrelated request, we hope
			fmt.Fprintf(os.Stderr, "spurious response: %s\n", string(body))
			continue
			// return nil, fmt.Errorf("wrong response: expected ID %d but got id %d", id, int(res["id"].(float64)))
		}
		if _, ok := res["error"]; ok {
			return nil, fmt.Errorf("error from LSP: %s", string(body))
		}
		return res["result"].(M), nil
	}
}

func (c *Client) ReadByte() (byte, error) {
	bs := [1]byte{}
	_, err := c.r.Read(bs[:])
	if err != nil {
		return 0, err
	}
	return bs[0], nil
}
