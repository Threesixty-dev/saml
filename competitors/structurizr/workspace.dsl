workspace "Cold storage monitoring" "Review the placement of the alarm decision." {
    model {
        operator = person "Cold-store operator"
        monitoring = softwareSystem "Cold storage monitoring" {
            probe = container "Temperature probe" "Produces temperature samples." "Sensor firmware" {
                properties {
                    "site" "edge"
                }
            }
            evaluator = container "Threshold evaluator" "Evaluates samples locally." "Edge process" {
                properties {
                    "site" "edge"
                    "requirement" "R1: local alarm survives WAN loss; R2: response <= 100 ms"
                }
            }
            alarm = container "Local alarm" "Sounds when a threshold is exceeded." "Alarm firmware" {
                properties {
                    "site" "edge"
                }
            }
            history = container "Telemetry history" "Stores readings for later inspection." "Cloud service" {
                properties {
                    "site" "cloud"
                }
            }
        }
        probe -> evaluator "Sends a temperature sample" "Local link" {
            tags "Critical"
        }
        evaluator -> alarm "Requests an audible alarm" "Local link" {
            tags "Critical"
        }
        evaluator -> history "Publishes telemetry independently of alarming" "WAN"
        operator -> history "Inspects temperature history" "HTTPS"
        alarm -> operator "Warns about a threshold breach" "Audible signal"
    }
    views {
        container monitoring "responsibilities" {
            include *
            autoLayout lr
        }
    }
}
