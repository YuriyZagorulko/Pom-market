export const STRATEGY_HARD_CLOSE_API_CONFIRMATION = "CLOSE STRATEGY";

export function StrategyHardCloseConfirmBody() {
  return (
    <ul className="modal-bullet-list">
      <li>All PAPER positions belonging to this strategy will be closed.</li>
      <li>You may realize a loss depending on market prices at execution.</li>
      <li>Close may be partial if liquidity or minimum order rules block some lots.</li>
      <li>Manual positions and other strategies on this account are not affected.</li>
    </ul>
  );
}
