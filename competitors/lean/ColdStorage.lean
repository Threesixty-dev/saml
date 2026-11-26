import Std

namespace ColdStorage

inductive Component where
  | probe | evaluator | alarm | history
  deriving DecidableEq, Repr

inductive Site where
  | edge | cloud
  deriving DecidableEq, Repr

structure Architecture where
  site : Component -> Site
  worstCaseMs : Component -> Nat
  probeToEvaluatorMs : Nat
  evaluatorToAlarmMs : Nat

-- The modeled alarm path is probe -> evaluator -> alarm. Telemetry is separate.
def LocalAlarm (m : Architecture) : Prop :=
  m.site .probe = .edge /\ m.site .evaluator = .edge /\ m.site .alarm = .edge

def alarmLatencyMs (m : Architecture) : Nat :=
  m.worstCaseMs .probe + m.probeToEvaluatorMs + m.worstCaseMs .evaluator +
    m.evaluatorToAlarmMs + m.worstCaseMs .alarm

def WithinBudget (m : Architecture) : Prop := alarmLatencyMs m <= 100

def Conforms (m : Architecture) : Prop := LocalAlarm m /\ WithinBudget m

instance (m : Architecture) : Decidable (Conforms m) := by
  unfold Conforms LocalAlarm WithinBudget
  infer_instance

def check (m : Architecture) : Bool := decide (Conforms m)

theorem check_correct (m : Architecture) : check m = true <-> Conforms m := by
  simp [check]

def baseline : Architecture where
  site
    | .history => .cloud
    | _ => .edge
  worstCaseMs
    | .probe => 10
    | .evaluator => 20
    | .alarm => 10
    | .history => 0
  probeToEvaluatorMs := 2
  evaluatorToAlarmMs := 2

theorem baseline_latency : alarmLatencyMs baseline = 44 := by decide
theorem baseline_valid : Conforms baseline := by decide

def cloudEvaluator : Architecture :=
  { baseline with site := fun c => if c = .evaluator then .cloud else baseline.site c }

theorem cloud_evaluator_rejected : check cloudEvaluator = false := by decide

-- Locality fails independently of the evaluator's supplied timing bound.
theorem cloud_evaluator_violates_locality (m : Architecture)
    (h : m.site .evaluator = .cloud) : Not (LocalAlarm m) := by
  intro hLocal
  have incompatible : Site.cloud = Site.edge := h.symm.trans hLocal.2.1
  cases incompatible

def slowEvaluator : Architecture :=
  { baseline with worstCaseMs := fun c => if c = .evaluator then 140 else baseline.worstCaseMs c }

theorem slow_evaluator_latency : alarmLatencyMs slowEvaluator = 164 := by decide
theorem slow_evaluator_rejected : check slowEvaluator = false := by decide

-- Telemetry timing is excluded under the explicit nonblocking assumption.
def slowHistory : Architecture :=
  { baseline with worstCaseMs := fun c => if c = .history then 1000 else baseline.worstCaseMs c }

theorem slow_history_valid : Conforms slowHistory := by decide

end ColdStorage
