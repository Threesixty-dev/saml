import Std

namespace ArticlePublication

inductive ActorId where
  | author
  deriving DecidableEq, Repr

inductive ArticleId where
  | submitted | other
  deriving DecidableEq, Repr

inductive DataId where
  | articleData
  deriving DecidableEq, Repr

inductive CapabilityId where
  | articlesApi | auditApi
  deriving DecidableEq, BEq, ReflBEq, LawfulBEq, Repr

inductive ApplicationId where
  | frontend | backend
  deriving DecidableEq, Repr

inductive Runtime where
  | browser | server
  deriving DecidableEq, Repr

inductive Stage where
  | evaluation | publication
  deriving DecidableEq, Repr

inductive Outcome where
  | successful | failed
  deriving DecidableEq, Repr

inductive QueueTechnology where
  | rabbitMQ
  deriving DecidableEq, Repr

structure Article where
  letters : Nat

structure DataObject where
  realizes : ArticleId

structure Capability where
  exposes : DataId

structure Application where
  runtime : Runtime
  provides : List CapabilityId

structure Consumption where
  consumer : ApplicationId
  provider : ApplicationId
  via : CapabilityId

structure Workflow where
  initiator : ActorId
  subject : ArticleId
  publicationAfter : Stage
  requiredOutcome : Outcome

structure PublicationQueue where
  technology : QueueTechnology
  producer : ApplicationId
  consumerStage : Stage

-- Separate ID types constrain relation endpoints; total tables resolve every ID.
structure Architecture where
  articles : ArticleId -> Article
  dataObjects : DataId -> DataObject
  capabilities : CapabilityId -> Capability
  applications : ApplicationId -> Application
  call : Consumption
  workflow : Workflow
  queue : PublicationQueue

def PublicationGate (m : Architecture) : Prop :=
  m.workflow.publicationAfter = .evaluation /\ m.workflow.requiredOutcome = .successful

def Conforms (m : Architecture) : Prop :=
  (m.applications m.call.consumer).runtime = .browser /\
  Membership.mem (m.applications m.call.provider).provides m.call.via /\
  (m.dataObjects (m.capabilities m.call.via).exposes).realizes = m.workflow.subject /\
  PublicationGate m /\
  m.queue.producer = m.call.provider /\ m.queue.consumerStage = .evaluation

instance (m : Architecture) : Decidable (Conforms m) := by
  unfold Conforms PublicationGate
  infer_instance

def check (m : Architecture) : Bool := decide (Conforms m)

theorem check_correct (m : Architecture) : check m = true <-> Conforms m := by
  simp [check]

def baseline : Architecture where
  articles := fun _ => { letters := 150 }
  dataObjects := fun _ => { realizes := .submitted }
  capabilities := fun _ => { exposes := .articleData }
  applications
    | .frontend => { runtime := .browser, provides := [] }
    | .backend => { runtime := .server, provides := [.articlesApi] }
  call := { consumer := .frontend, provider := .backend, via := .articlesApi }
  workflow := {
    initiator := .author, subject := .submitted,
    publicationAfter := .evaluation, requiredOutcome := .successful
  }
  queue := { technology := .rabbitMQ, producer := .backend, consumerStage := .evaluation }

theorem baseline_valid : Conforms baseline := by decide

structure VerifiedArchitecture where
  model : Architecture
  valid : Conforms model

def verifiedBaseline : VerifiedArchitecture := { model := baseline, valid := baseline_valid }

def missingProvider : Architecture :=
  { baseline with call := { baseline.call with via := .auditApi } }

def unrelatedData : Architecture :=
  { baseline with dataObjects := fun _ => { realizes := .other } }

def ungatedPublication : Architecture :=
  { baseline with workflow := { baseline.workflow with requiredOutcome := .failed } }

theorem missing_provider_rejected : check missingProvider = false := by decide
theorem unrelated_data_rejected : check unrelatedData = false := by decide
theorem ungated_publication_rejected : check ungatedPublication = false := by decide

-- At least 100 letters is an example policy, not a requirement supplied by SAML.
def Eligible (m : Architecture) : Prop := 100 <= (m.articles m.workflow.subject).letters

inductive State where
  | submitted | successful | failed | published
  deriving DecidableEq, Repr

inductive Step (m : Architecture) : State -> State -> Prop where
  | evaluateSuccess : Eligible m -> Step m .submitted .successful
  | evaluateFailure : Not (Eligible m) -> Step m .submitted .failed
  | publish : PublicationGate m -> Step m .successful .published

inductive Reachable (m : Architecture) : State -> Prop where
  | initial : Reachable m .submitted
  | next : Reachable m before -> Step m before after -> Reachable m after

def StateInvariant (m : Architecture) : State -> Prop
  | .successful | .published => Eligible m
  | .submitted | .failed => True

theorem step_preserves_invariant {m : Architecture} {before after : State}
    (step : Step m before after) (safe : StateInvariant m before) :
    StateInvariant m after := by
  cases step with
  | evaluateSuccess eligible => exact eligible
  | evaluateFailure _ => trivial
  | publish _ => exact safe

theorem reachable_preserves_invariant {m : Architecture} {state : State}
    (reachable : Reachable m state) : StateInvariant m state := by
  induction reachable with
  | initial => trivial
  | next _ step ih => exact step_preserves_invariant step ih

theorem publication_requires_eligible {m : Architecture}
    (published : Reachable m .published) : Eligible m :=
  reachable_preserves_invariant published

theorem publication_requires_successful_evaluation {m : Architecture}
    (published : Reachable m .published) : Reachable m .successful := by
  cases published with
  | next previous step =>
    cases step with
    | publish _ => exact previous

theorem baseline_can_publish : Reachable baseline .published :=
  .next (.next .initial (.evaluateSuccess (by unfold Eligible; decide)))
    (.publish (by unfold PublicationGate; decide))

def thresholdArticle : Architecture :=
  { baseline with articles := fun _ => { letters := 100 } }

theorem threshold_article_can_publish : Reachable thresholdArticle .published :=
  .next (.next .initial (.evaluateSuccess (by unfold Eligible; decide)))
    (.publish (by unfold PublicationGate; decide))

def shortArticle : Architecture :=
  { baseline with articles := fun _ => { letters := 99 } }

-- Invalid content is rejected by the workflow, not by the architecture schema.
theorem short_article_architecture_valid : Conforms shortArticle := by decide

theorem short_article_can_fail : Reachable shortArticle .failed :=
  .next .initial (.evaluateFailure (by unfold Eligible; decide))

theorem short_article_cannot_publish : Not (Reachable shortArticle .published) := by
  intro published
  have ineligible : Not (Eligible shortArticle) := by unfold Eligible; decide
  exact ineligible (publication_requires_eligible published)

end ArticlePublication
