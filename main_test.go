package main

import (
	"bytes"
	"strings"
	"testing"
)

func TestRunAppVersion(t *testing.T) {
	// -version takes precedence over every output mode: whichever other
	// flags are set, the output is exactly one version line and exit 0.
	tests := []struct {
		name string
		args []string
	}{
		{"version alone", []string{"-version"}},
		{"version with json and n", []string{"-version", "-json", "-n", "5"}},
		{"version with pretty", []string{"-version", "-pretty"}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var buf bytes.Buffer
			if code := runApp(&buf, tt.args); code != 0 {
				t.Fatalf("runApp(%v) exit code = %d, want 0", tt.args, code)
			}
			want := "gofib " + Version + "\n"
			if got := buf.String(); got != want {
				t.Errorf("runApp(%v) output = %q, want exactly %q", tt.args, got, want)
			}
			if n := strings.Count(buf.String(), "\n"); n != 1 {
				t.Errorf("runApp(%v) printed %d newlines, want 1", tt.args, n)
			}
		})
	}
}

func TestRunAppDispatch(t *testing.T) {
	// Guard the runApp refactor: normal flag combinations still dispatch
	// to run() with the parsed values and exit 0.
	tests := []struct {
		name string
		args []string
		want string
	}{
		{"text n=3", []string{"-n", "3"}, "1: 1\n2: 1\n3: 2\n"},
		{"json n=2", []string{"-json", "-n", "2"},
			`{"index":1,"fib":"1"}` + "\n" + `{"index":2,"fib":"1"}` + "\n"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var buf bytes.Buffer
			if code := runApp(&buf, tt.args); code != 0 {
				t.Fatalf("runApp(%v) exit code = %d, want 0", tt.args, code)
			}
			if got := buf.String(); got != tt.want {
				t.Errorf("runApp(%v) output = %q, want %q", tt.args, got, tt.want)
			}
		})
	}

	// Invalid -n still exits 1 with the run() error message.
	var buf bytes.Buffer
	if code := runApp(&buf, []string{"-n", "0"}); code != 1 {
		t.Fatalf("runApp(-n 0) exit code = %d, want 1", code)
	}
	if buf.Len() != 0 {
		t.Errorf("runApp(-n 0) wrote %q to stdout, want none", buf.String())
	}
}
