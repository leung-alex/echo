import type { ActivationPayload } from "./generated";
import { invoke } from "./invoke";

export function getActivationState(): Promise<ActivationPayload | null> {
  return invoke<ActivationPayload | null>("activation_state");
}

export function acknowledgeActivation(requestId: string): Promise<void> {
  return invoke<void>("activation_ack", { request_id: requestId });
}
