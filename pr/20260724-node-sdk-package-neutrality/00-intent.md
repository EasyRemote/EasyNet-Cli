# Intent

Remove the EasyNet-branded Node SDK package identity from the canonical runtime
SDK seam.

The Node package is private and already exposes only generic runtime concepts,
but its package metadata still identifies it as `@easynet/daemon-sdk`. That
metadata teaches downstream products and plugins to depend on an EasyNet
package when they should depend on the canonical runtime SDK.

This iteration renames the private package metadata to a product-neutral
runtime identity and updates the gate that previously enforced the product
name.
