workspace "Article publication" "Applications and workflow for publishing an article." {
    model {
        author = person "Author" "Submits an article for publication."

        publishing = softwareSystem "Publishing platform" {
            frontend = container "User portal frontend" "Browser-based author interface." "Angular SPA" {
                properties {
                    "runtime" "browser"
                }
            }
            backend = container "User portal backend" "Exposes article operations." "Express" {
                api = component "Articles REST API" "Accepts and exposes article data." "HTTP/REST"
                evaluation = component "Evaluation" "Checks the article rule." "Workflow stage"
                publication = component "Publication" "Publishes an approved article." "Workflow stage"
            }
            queue = container "Article publication queue" "Holds publication requests." "RabbitMQ"
            store = container "Article store" "Persists Article and Author data." "Database"
        }

        author -> frontend "Submits and views articles" "Browser UI"
        frontend -> api "Consumes article operations" "HTTPS/REST"
        api -> queue "Enqueues publication requests" "Message"
        queue -> evaluation "Delivers a request" "Message"
        evaluation -> publication "Passes an approved article" "Workflow transition"
        api -> store "Reads Article and Author data" "Database access"
        publication -> store "Writes the published article" "Database access"
    }

    views {
        container publishing "applications" {
            include *
            autoLayout lr
        }
        component backend "publication-workflow" {
            include *
            autoLayout lr
        }
    }
}
