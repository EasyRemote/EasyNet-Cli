"""DaemonHandle profile-client factory mixin."""

from __future__ import annotations

from .ability_invocation import AbilityInvocationClient
from .admin import AdminClient
from .compatibility import CompatibilityClient
from .connection import ConnectOptions
from .directory import DirectoryClient
from .events import EventClient
from .health import HealthClient
from .host_binding import HostBindingClient
from .identity import AddressingClient, IdentityClient
from .mission import MissionClient
from .publication import PublicationClient, RuntimePublicationTransport
from .receipt import ReceiptClient
from .runtime import RuntimeClient
from .surface import SurfaceClient
from .wrappers import WrapperClient


class DaemonHandleProfiles:
    """Typed profile factories mixed into the daemon lifecycle handle."""

    def directory(self, options: ConnectOptions = ConnectOptions()) -> DirectoryClient:
        """Open a Directory profile client scoped to this daemon handle."""

        return DirectoryClient(self._open_profile("directory", options))

    def identity(self, options: ConnectOptions = ConnectOptions()) -> IdentityClient:
        """Open an Identity profile client scoped to this daemon handle."""

        return IdentityClient(self._open_profile("identity", options))

    def addressing(
        self, options: ConnectOptions = ConnectOptions()
    ) -> AddressingClient:
        """Open the Axon-delegated addressing helper facade."""

        return AddressingClient(self._open_profile("identity", options))

    def receipts(self, options: ConnectOptions = ConnectOptions()) -> ReceiptClient:
        """Open a Receipt profile client scoped to this daemon handle."""

        return ReceiptClient(self._open_profile("receipt", options))

    def publication(
        self, options: ConnectOptions = ConnectOptions()
    ) -> PublicationClient:
        """Open a Publication profile client scoped to this daemon handle."""

        carrier = self._open_profile("publication", options)
        runtime_transport = self._open_profile("runtime", options)
        return PublicationClient(
            RuntimePublicationTransport(
                carrier=carrier,
                runtime=RuntimeClient(runtime_transport),
            )
        )

    def host_binding(
        self, options: ConnectOptions = ConnectOptions()
    ) -> HostBindingClient:
        """Open a Host Binding profile client scoped to this daemon handle."""

        return HostBindingClient(self._open_profile("host_binding", options))

    def missions(self, options: ConnectOptions = ConnectOptions()) -> MissionClient:
        """Open a Mission profile client scoped to this daemon handle."""

        return MissionClient(self._open_profile("mission", options))

    def admin(self, options: ConnectOptions = ConnectOptions()) -> AdminClient:
        """Open an Admin + Gateway profile client scoped to this daemon handle."""

        return AdminClient(self._open_profile("admin", options))

    def events(self, options: ConnectOptions = ConnectOptions()) -> EventClient:
        """Open an Events profile client scoped to this daemon handle."""

        return EventClient(self._open_profile("events", options))

    def surfaces(self, options: ConnectOptions = ConnectOptions()) -> SurfaceClient:
        """Open a Surface profile client scoped to this daemon handle."""

        return SurfaceClient(self._open_profile("surface", options))

    def compatibility(
        self, options: ConnectOptions = ConnectOptions()
    ) -> CompatibilityClient:
        """Open a Compatibility profile client scoped to this daemon handle."""

        return CompatibilityClient(self._open_profile("compatibility", options))

    def wrappers(self, options: ConnectOptions = ConnectOptions()) -> WrapperClient:
        """Open a Convenience Wrapper profile client scoped to this daemon handle."""

        return WrapperClient(self._open_profile("wrapper", options))

    def health(self, options: ConnectOptions = ConnectOptions()) -> HealthClient:
        """Open a runtime health client scoped to this daemon handle."""

        return HealthClient(self._open_profile("runtime", options))

    def ability_invocation(
        self, options: ConnectOptions = ConnectOptions()
    ) -> AbilityInvocationClient:
        """Open the ability Invocation convenience facade for this daemon handle."""

        return AbilityInvocationClient(
            runtime=RuntimeClient(self._open_profile("runtime", options)),
            addressing=AddressingClient(self._open_profile("identity", options)),
        )
