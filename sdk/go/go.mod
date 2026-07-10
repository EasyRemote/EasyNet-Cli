module easynet.run/cli/sdk/go

go 1.22

require (
	easynet.run/axon/sdk/go v0.0.0
	golang.org/x/crypto v0.21.0
	google.golang.org/grpc v1.64.0
	google.golang.org/protobuf v1.34.2
)

require (
	golang.org/x/net v0.22.0 // indirect
	golang.org/x/sys v0.18.0 // indirect
	golang.org/x/text v0.14.0 // indirect
	google.golang.org/genproto/googleapis/rpc v0.0.0-20240318140521-94a12d6c2237 // indirect
)

replace easynet.run/axon/sdk/go => ../../../EasyNet-Axon/sdk/go
