package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func write(t *testing.T, body string) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "config.toml")
	if err := os.WriteFile(path, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestLoadMissingFileIsEmpty(t *testing.T) {
	f, err := Load(filepath.Join(t.TempDir(), "absent.toml"))
	if err != nil || f.Exists || f.Err != nil {
		t.Fatalf("Load = %+v, %v; want empty and no error", f, err)
	}
	for _, s := range f.Settings() {
		if s.Source != "default" {
			t.Errorf("%s from %s, want default", s.Key, s.Source)
		}
	}
}

func TestLoadValues(t *testing.T) {
	t.Setenv("HOME", "/home/x")
	t.Setenv("CASES_STORE", "")
	f, err := Load(write(t, "store = \"~/notes/cases\"\nlisten = \"localhost:9000\"\n"))
	if err != nil {
		t.Fatal(err)
	}
	got := map[string]Setting{}
	for _, s := range f.Settings() {
		got[s.Key] = s
	}
	if s := got["store"]; s.Value != "/home/x/notes/cases" || s.Source != "config" {
		t.Errorf("store = %+v", s)
	}
	if s := got["listen"]; s.Value != "localhost:9000" || s.Source != "config" {
		t.Errorf("listen = %+v", s)
	}

	t.Setenv("CASES_STORE", "/elsewhere")
	for _, s := range f.Settings() {
		if s.Key == "store" && (s.Value != "/elsewhere" || s.Source != "env") {
			t.Errorf("store with CASES_STORE set = %+v, want env", s)
		}
	}
}

func TestLoadRejectsBadFiles(t *testing.T) {
	tests := []struct{ name, body, want string }{
		{"unknown key", "stor = \"x\"\n", `unknown key "stor" (valid keys: store, listen, no-open, prune-age)`},
		{"wrong type", "store = true\n", `key "store" must be a string, got boolean`},
		{"malformed", "store = \n", "invalid TOML: line 1"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			path := write(t, tt.body)
			f, err := Load(path)
			if err == nil || !strings.Contains(err.Error(), tt.want) || !strings.Contains(err.Error(), path) {
				t.Fatalf("err = %v, want %q naming the file", err, tt.want)
			}
			if !f.Exists || f.Err == nil || len(f.values) != 0 {
				t.Errorf("File = %+v, want exists, failed, no values", f)
			}
		})
	}
	dir := t.TempDir()
	if f, err := Load(dir); err == nil || !f.Exists || !strings.Contains(err.Error(), "cannot read") {
		t.Errorf("Load(directory) = %+v, %v", f, err)
	}
}

func TestPaths(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", "/xdg")
	t.Setenv("XDG_DATA_HOME", "/xdgdata")
	t.Setenv("HOME", "/home/x")
	t.Setenv(EnvVar, "")
	if p, _ := DefaultPath(); p != "/xdg/cases/config.toml" {
		t.Errorf("DefaultPath = %s", p)
	}
	if p := DefaultStore(); p != "/xdgdata/cases" {
		t.Errorf("DefaultStore = %s", p)
	}
	t.Setenv("XDG_CONFIG_HOME", "")
	t.Setenv("XDG_DATA_HOME", "")
	if p, _ := DefaultPath(); p != "/home/x/.config/cases/config.toml" {
		t.Errorf("DefaultPath = %s", p)
	}
	if p := DefaultStore(); p != "/home/x/.local/share/cases" {
		t.Errorf("DefaultStore = %s", p)
	}
	if p, src, _ := ResolvePath(""); p != "/home/x/.config/cases/config.toml" || src != SourceDefault {
		t.Errorf("ResolvePath = %s, %s", p, src)
	}
	t.Setenv(EnvVar, "~/env.toml")
	if p, src, _ := ResolvePath(""); p != "/home/x/env.toml" || src != SourceEnv {
		t.Errorf("ResolvePath = %s, %s", p, src)
	}
	if p, src, _ := ResolvePath("~/flag.toml"); p != "/home/x/flag.toml" || src != SourceFlag {
		t.Errorf("ResolvePath = %s, %s", p, src)
	}
	t.Setenv("HOME", "")
	if _, err := DefaultPath(); err == nil {
		t.Error("DefaultPath with no HOME: want error")
	}
	if p := DefaultStore(); p != "" {
		t.Errorf("DefaultStore with no HOME = %q", p)
	}
	if p := ExpandHome("~/x"); p != "~/x" {
		t.Errorf("ExpandHome with no HOME = %q", p)
	}
}

func TestTemplateLoads(t *testing.T) {
	f, err := Load(write(t, Template()))
	if err != nil {
		t.Fatal(err)
	}
	if len(f.values) != 0 {
		t.Errorf("template sets %v; every line should be commented out", f.values)
	}
	for _, k := range Keys {
		if !strings.Contains(Template(), "# "+k.Example) {
			t.Errorf("template lacks the example for %s", k.Name)
		}
	}
}

// TestLoadNamesWrongTypes gives every key a value of each TOML type that is
// not a string, and checks the error names the type as TOML spells it, or
// as the decoder's Go type for the date and time forms.
func TestLoadNamesWrongTypes(t *testing.T) {
	values := []struct{ toml, want string }{
		{"true", "boolean"},
		{"42", "number"},
		{"1.5", "number"},
		{"inf", "number"},
		{"{ a = \"b\" }", "table"},
		{"[\"a\", \"b\"]", "array"},
		{"1979-05-27T07:32:00Z", "time.Time"},
		{"1979-05-27T07:32:00", "toml.LocalDateTime"},
		{"1979-05-27", "toml.LocalDate"},
		{"07:32:00", "toml.LocalTime"},
	}
	for _, k := range Keys {
		for _, v := range values {
			t.Run(k.Name+"="+v.toml, func(t *testing.T) {
				_, err := Load(write(t, k.Name+" = "+v.toml+"\n"))
				want := `key "` + k.Name + `" must be a string, got ` + v.want
				if err == nil || !strings.HasSuffix(err.Error(), want) {
					t.Errorf("err = %v, want %q", err, want)
				}
			})
		}
	}
	// A table header decodes to the same map as an inline table.
	if _, err := Load(write(t, "[store]\npath = \"x\"\n")); err == nil || !strings.HasSuffix(err.Error(), "got table") {
		t.Errorf("[store] table: err = %v", err)
	}
}
