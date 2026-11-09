export type EmailDelivery = {
  configured: boolean;
  sent: boolean;
  detail: string;
};

export type InviteResult = {
  setup_url?: string;
  already_exists?: boolean;
  email?: string;
  email_delivery?: EmailDelivery;
};

export function inviteFeedback(result: InviteResult): { message: string; ok: boolean } {
  if (result.email_delivery?.detail) {
    return { message: result.email_delivery.detail, ok: result.email_delivery.sent };
  }
  if (result.setup_url) {
    return {
      message: "Invitation created. Email delivery status is unknown; copy the link below if needed.",
      ok: true,
    };
  }
  if (result.already_exists) {
    return {
      message: `${result.email ?? "This user"} already has an account. Open their profile to send a new invitation.`,
      ok: false,
    };
  }
  return { message: "Done.", ok: true };
}
