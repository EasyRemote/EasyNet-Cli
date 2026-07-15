package easynet

import "testing"

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
