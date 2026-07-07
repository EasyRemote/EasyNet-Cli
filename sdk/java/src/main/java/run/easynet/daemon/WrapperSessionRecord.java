package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

abstract class WrapperSessionRecord {
  final String profile;
  final String kind;
  final String sessionID;
  final String ownerURA;
  final String state;
  final String refField;
  final String refValue;
  final Map<String, Object> metadata;

  WrapperSessionRecord(
      String profile,
      String kind,
      String expectedKind,
      String sessionID,
      String ownerURA,
      String state,
      String refField,
      String refValue,
      Map<String, Object> metadata) {
    WrapperSupport.validateKind(profile, kind, expectedKind);
    this.profile = profile;
    this.kind = kind;
    this.sessionID = WrapperSupport.requiredString(sessionID, "session_id");
    this.ownerURA = WrapperSupport.requiredURA(ownerURA, "owner_ura");
    this.state = WrapperSupport.requiredString(state, "state");
    this.refField = refField;
    this.refValue = WrapperSupport.optionalString(refValue, refField);
    this.metadata = WrapperSupport.copyObject(metadata);
  }

  Map<String, Object> toObject() {
    LinkedHashMap<String, Object> object = new LinkedHashMap<>();
    object.put("profile", profile);
    object.put("kind", kind);
    object.put("session_id", sessionID);
    object.put("owner_ura", ownerURA);
    object.put("state", state);
    object.put(refField, refValue);
    object.put("metadata", metadata);
    return object;
  }
}
