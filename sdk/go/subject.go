package easynet

import "context"

// DescriptorBoundResourceSubjectURA asks the daemon/Axon identity boundary for
// the canonical resource URA used as a descriptor-bound Invocation subject.
func (c *IdentityClient) DescriptorBoundResourceSubjectURA(ctx context.Context, ownerURA string, path string) (string, error) {
	return c.ResourceURA(ctx, ownerURA, path)
}
