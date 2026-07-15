# Continuation benchmark recommendation

- Completed measured arms: 6 / 864
- Overall recommendation: `no_auto_rollover`
- Recommendations are model-specific; untested model snapshots receive no threshold.

## gpt-5.5

- Result: `no_auto_rollover`
- Completed arms: 6
- Base matrix: 6 / 432
- Degradation boundary: not established (no metric crossed its point threshold and adverse-side 95% CI at two consecutive checkpoints; isolated or uncertain spikes were not adopted)
- Product arm gate: not passed or unavailable
- Reason: the required base measured matrix is incomplete for this model
- Reason: no confirmed two-checkpoint degradation boundary

### Paired same-task vs native-handoff comparison

All differences use `native_handoff - same_task`; positive latency means native handoff is slower.

| Checkpoint | Pairs | Accuracy pp | Serious error pp | Completion % | E2E % | Goal pp | Files pp | Tests pp | Next step pp |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | 3/24 | unavailable | unavailable | unavailable | unavailable | unavailable | unavailable | unavailable | unavailable |

## Statistical limitations

- Each model/scenario/strategy/checkpoint condition has only n=3 repeats; confidence intervals are low-power and may be wide.
- Scenario-repeat arms are paired for comparisons, but the eight scenarios are not assumed to be interchangeable population samples.
- Bootstrap intervals describe this fixed benchmark matrix and must not be generalized to untested model snapshots or versions.
