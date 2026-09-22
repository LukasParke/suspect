package negative

import generated "example.com/terraform-provider-fixture/provider"

// The native public provider factory requires a version string.
var _ = generated.NewWithTransport(42, nil)
