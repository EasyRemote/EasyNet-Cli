# Intent

Remove the remote invocation compatibility projection that turns unknown wire states into product-visible `UNKNOWN_STATE_*` strings.

Unknown states are not runtime business states. They indicate a protocol/schema mismatch and must fail closed at the remote invocation adapter boundary before product code can render or branch on them.
