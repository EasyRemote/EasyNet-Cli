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

        factory = self._require_profile_factory()
        return DirectoryClient(
            self._open_profile("directory", factory.open_directory_transport, options)
        )

    def identity(self, options: ConnectOptions = ConnectOptions()) -> IdentityClient:
        """Open an Identity profile client scoped to this daemon handle."""

        factory = self._require_profile_factory()
        return IdentityClient(
            self._open_profile("identity", factory.open_identity_transport, options)
        )

    def addressing(
        self, options: ConnectOptions = ConnectOptions()
    ) -> AddressingClient:
        """Open the Axon-delegated addressing helper facade."""

        factory = self._require_profile_factory()
        return AddressingClient(
            self._open_profile("identity", factory.open_identity_transport, options)
        )

    def receipts(self, options: ConnectOptions = ConnectOptions()) -> ReceiptClient:
        """Open a Receipt profile client scoped to this daemon handle."""

        factory = self._require_profile_factory()
        return ReceiptClient(
            self._open_profile("receipt", factory.open_receipt_transport, options)
        )

    def publication(
        self, options: ConnectOptions = ConnectOptions()
    ) -> PublicationClient:
        """Open a Publication profile client scoped to this daemon handle."""

        factory = self._require_profile_factory()
        carrier = self._open_profile(
            "publication", factory.open_publication_transport, options
        )
        runtime_transport = self._open_profile(
            "runtime", factory.open_runtime_transport, options
        )
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

        factory = self._require_profile_factory()
        return HostBindingClient(
            self._open_profile(
                "host_binding", factory.open_host_binding_transport, options
            )
        )

    def missions(self, options: ConnectOptions = ConnectOptions()) -> MissionClient:
        """Open a Mission profile client scoped to this daemon handle."""

        factory = self._require_profile_factory()
        return MissionClient(
            self._open_profile("mission", factory.open_mission_transport, options)
        )

    def admin(self, options: ConnectOptions = ConnectOptions()) -> AdminClient:
        """Open an Admin + Gateway profile client scoped to this daemon handle."""

        factory = self._require_profile_factory()
        return AdminClient(
            self._open_profile("admin", factory.open_admin_transport, options)
        )

    def events(self, options: ConnectOptions = ConnectOptions()) -> EventClient:
        """Open an Events profile client scoped to this daemon handle."""

        factory = self._require_profile_factory()
        return EventClient(
            self._open_profile("events", factory.open_events_transport, options)
        )

    def surfaces(self, options: ConnectOptions = ConnectOptions()) -> SurfaceClient:
        """Open a Surface profile client scoped to this daemon handle."""

        factory = self._require_profile_factory()
        return SurfaceClient(
            self._open_profile("surface", factory.open_surface_transport, options)
        )

    def compatibility(
        self, options: ConnectOptions = ConnectOptions()
    ) -> CompatibilityClient:
        """Open a Compatibility profile client scoped to this daemon handle."""

        factory = self._require_profile_factory()
        return CompatibilityClient(
            self._open_profile(
                "compatibility", factory.open_compatibility_transport, options
            )
        )

    def wrappers(self, options: ConnectOptions = ConnectOptions()) -> WrapperClient:
        """Open a Convenience Wrapper profile client scoped to this daemon handle."""

        factory = self._require_profile_factory()
        return WrapperClient(
            self._open_profile("wrapper", factory.open_wrapper_transport, options)
        )

    def health(self, options: ConnectOptions = ConnectOptions()) -> HealthClient:
        """Open a runtime health client scoped to this daemon handle."""

        factory = self._require_profile_factory()
        return HealthClient(
            self._open_profile("runtime", factory.open_runtime_transport, options)
        )

    def ability_invocation(
        self, options: ConnectOptions = ConnectOptions()
    ) -> AbilityInvocationClient:
        """Open the ability Invocation convenience facade for this daemon handle."""

        factory = self._require_profile_factory()
        return AbilityInvocationClient(
            runtime=RuntimeClient(
                self._open_profile("runtime", factory.open_runtime_transport, options)
            ),
            addressing=AddressingClient(
                self._open_profile("identity", factory.open_identity_transport, options)
            ),
        )
