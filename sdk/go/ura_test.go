package easynet

import "testing"

func TestResourceURAPreservesRuntimeNamespaceProjection(t *testing.T) {
	got := ResourceURA("example", "alice", string(ResourceNamespaceFS), "/tmp/a.txt")
	want := "easynet:///r/example/resource/alice/fs/tmp/a.txt"
	if got != want {
		t.Fatalf("ResourceURA() = %q, want %q", got, want)
	}

	parsed, err := ParseURAParts(got)
	if err != nil {
		t.Fatalf("ParseURAParts() error = %v", err)
	}
	if parsed.ResourceNamespace != ResourceNamespaceFS {
		t.Fatalf("ResourceNamespace = %q, want %q", parsed.ResourceNamespace, ResourceNamespaceFS)
	}
	if parsed.Path != "tmp/a.txt" {
		t.Fatalf("Path = %q, want tmp/a.txt", parsed.Path)
	}
}

func TestResourceURARejectsUnknownRuntimeNamespace(t *testing.T) {
	if got := ResourceURA("example", "alice", "product-private", "tmp/a.txt"); got != "" {
		t.Fatalf("ResourceURA() with unknown namespace = %q, want empty", got)
	}
	if IsResourceNamespace("product-private") {
		t.Fatal("IsResourceNamespace accepted product-private")
	}
}

func TestRuntimeResourceURAPreservesCanonicalWireForm(t *testing.T) {
	tests := []struct {
		name string
		path string
		want string
	}{
		{
			name: "resource root",
			path: "",
			want: "easynet:///r/example/resource/alice.console",
		},
		{
			name: "explicit resource root",
			path: "/",
			want: "easynet:///r/example/resource/alice.console/",
		},
		{
			name: "relative path",
			path: "assets/app.js",
			want: "easynet:///r/example/resource/alice.console/assets/app.js",
		},
		{
			name: "absolute path",
			path: "/assets/app.js",
			want: "easynet:///r/example/resource/alice.console/assets/app.js",
		},
	}

	for _, testCase := range tests {
		t.Run(testCase.name, func(t *testing.T) {
			got := RuntimeResourceURA("example", "alice", "console", testCase.path)
			if got != testCase.want {
				t.Fatalf("RuntimeResourceURA() = %q, want %q", got, testCase.want)
			}
		})
	}
}
