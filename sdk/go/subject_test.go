package easynet

import (
	"testing"
)

func TestDescriptorBoundResourceSubjectURA(t *testing.T) {
	got, err := DescriptorBoundResourceSubjectURA("example", "user.alice", "/invoke/files.read")
	if err != nil {
		t.Fatalf("DescriptorBoundResourceSubjectURA: %v", err)
	}
	want := "easynet:///r/example/resource/user.alice/invoke/files.read"
	if got != want {
		t.Fatalf("subject URA = %q, want %q", got, want)
	}
}

func TestDescriptorBoundResourceSubjectURARejectsInvalidInputs(t *testing.T) {
	cases := []struct {
		name    string
		realm   string
		ownerID string
		path    string
	}{
		{name: "empty realm", realm: "", ownerID: "user.alice", path: "invoke/files.read"},
		{name: "empty owner", realm: "example", ownerID: "", path: "invoke/files.read"},
		{name: "empty path", realm: "example", ownerID: "user.alice", path: ""},
		{name: "slash in realm", realm: "bad/realm", ownerID: "user.alice", path: "invoke/files.read"},
		{name: "slash in owner", realm: "example", ownerID: "user/alice", path: "invoke/files.read"},
		{name: "empty path segment", realm: "example", ownerID: "user.alice", path: "invoke//files.read"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if _, err := DescriptorBoundResourceSubjectURA(tc.realm, tc.ownerID, tc.path); !IsCode(err, ErrInvalidArgument) {
				t.Fatalf("DescriptorBoundResourceSubjectURA error = %v, want invalid argument", err)
			}
		})
	}
}
