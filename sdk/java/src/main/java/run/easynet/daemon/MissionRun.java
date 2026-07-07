package run.easynet.daemon;

public record MissionRun(MissionStatus status) {
  public MissionRun {
    if (status == null) {
      throw MissionSupport.invalid("status is required");
    }
  }
}
