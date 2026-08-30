//! Explicit lifecycle for one daemon-owned Runtime admission reservation.
//!
//! Axon owns descriptor verification, caller signatures, nonce replay, and
//! canonical receipts. This state machine owns only the product-policy facts
//! staged beside that canonical pipeline and the provisional quota reservation
//! that must be committed after Axon admits, or rolled back on every other
//! exit.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AdmissionTransactionPhase {
    Staged,
    Evaluating,
    Reserved,
    Denied,
}

pub(super) enum AdmissionTransaction<R> {
    Staged,
    Evaluating,
    Reserved(R),
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AdmissionTransitionError {
    InvalidPhase {
        expected: AdmissionTransactionPhase,
        actual: AdmissionTransactionPhase,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AdmissionCommitError {
    Denied,
    Incomplete(AdmissionTransactionPhase),
}

pub(super) enum AdmissionTransactionFinalization<R> {
    Committed(R),
    RolledBack,
}

impl<R> AdmissionTransaction<R> {
    pub(super) fn staged() -> Self {
        Self::Staged
    }

    pub(super) fn phase(&self) -> AdmissionTransactionPhase {
        match self {
            Self::Staged => AdmissionTransactionPhase::Staged,
            Self::Evaluating => AdmissionTransactionPhase::Evaluating,
            Self::Reserved(_) => AdmissionTransactionPhase::Reserved,
            Self::Denied => AdmissionTransactionPhase::Denied,
        }
    }

    pub(super) fn is_staged(&self) -> bool {
        self.phase() == AdmissionTransactionPhase::Staged
    }

    pub(super) fn begin_evaluation(&mut self) -> Result<(), AdmissionTransitionError> {
        match self {
            Self::Staged => {
                *self = Self::Evaluating;
                Ok(())
            }
            Self::Evaluating | Self::Reserved(_) | Self::Denied => {
                Err(AdmissionTransitionError::InvalidPhase {
                    expected: AdmissionTransactionPhase::Staged,
                    actual: self.phase(),
                })
            }
        }
    }

    pub(super) fn reserve(&mut self, reservation: R) -> Result<(), AdmissionTransitionError> {
        if !matches!(self, Self::Evaluating) {
            return Err(AdmissionTransitionError::InvalidPhase {
                expected: AdmissionTransactionPhase::Evaluating,
                actual: self.phase(),
            });
        }
        *self = Self::Reserved(reservation);
        Ok(())
    }

    pub(super) fn deny(&mut self) -> Result<(), AdmissionTransitionError> {
        if !matches!(self, Self::Evaluating) {
            return Err(AdmissionTransitionError::InvalidPhase {
                expected: AdmissionTransactionPhase::Evaluating,
                actual: self.phase(),
            });
        }
        *self = Self::Denied;
        Ok(())
    }

    pub(super) fn finish(
        self,
        commit: bool,
    ) -> Result<AdmissionTransactionFinalization<R>, AdmissionCommitError> {
        if !commit {
            return Ok(AdmissionTransactionFinalization::RolledBack);
        }
        match self {
            Self::Reserved(reservation) => {
                Ok(AdmissionTransactionFinalization::Committed(reservation))
            }
            Self::Denied => Err(AdmissionCommitError::Denied),
            Self::Staged => Err(AdmissionCommitError::Incomplete(
                AdmissionTransactionPhase::Staged,
            )),
            Self::Evaluating => Err(AdmissionCommitError::Incomplete(
                AdmissionTransactionPhase::Evaluating,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admitted_transaction_commits_its_exact_reservation() {
        let mut transaction = AdmissionTransaction::staged();
        assert_eq!(transaction.phase(), AdmissionTransactionPhase::Staged);
        transaction.begin_evaluation().expect("begin evaluation");
        transaction.reserve("quota-lease").expect("reserve quota");

        match transaction.finish(true).expect("commit admission") {
            AdmissionTransactionFinalization::Committed(reservation) => {
                assert_eq!(reservation, "quota-lease")
            }
            AdmissionTransactionFinalization::RolledBack => panic!("admission must commit"),
        }
    }

    #[test]
    fn rollback_is_total_for_every_non_committed_phase() {
        for transaction in [
            AdmissionTransaction::<()>::Staged,
            AdmissionTransaction::Evaluating,
            AdmissionTransaction::Reserved(()),
            AdmissionTransaction::Denied,
        ] {
            assert!(matches!(
                transaction.finish(false),
                Ok(AdmissionTransactionFinalization::RolledBack)
            ));
        }
    }

    #[test]
    fn reservation_cannot_skip_evaluation() {
        let mut transaction = AdmissionTransaction::staged();
        assert_eq!(
            transaction.reserve("quota-lease"),
            Err(AdmissionTransitionError::InvalidPhase {
                expected: AdmissionTransactionPhase::Evaluating,
                actual: AdmissionTransactionPhase::Staged,
            })
        );
        assert_eq!(transaction.phase(), AdmissionTransactionPhase::Staged);
    }

    #[test]
    fn commit_rejects_denied_or_incomplete_transactions() {
        assert!(matches!(
            AdmissionTransaction::<()>::Denied.finish(true),
            Err(AdmissionCommitError::Denied)
        ));
        assert!(matches!(
            AdmissionTransaction::<()>::Staged.finish(true),
            Err(AdmissionCommitError::Incomplete(
                AdmissionTransactionPhase::Staged
            ))
        ));
    }
}
