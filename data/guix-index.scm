;;; guixvis — read-only Guix package index generator.
;;; Usage: guix repl -- data/guix-index.scm [GUIX_COMMIT] [GENERATED_MS]
;;; One JSON document on stdout; progress only on stderr. No store/network writes.
(use-modules (gnu packages))
(unless (defined? 'collect-package-graph)
  (load (string-append (dirname (current-filename)) "/guix-index-core.scm")))
(let* ((args (cdr (command-line)))
       (commit (if (pair? args) (car args) ""))
       (generated-ms (if (and (pair? args) (pair? (cdr args))) (cadr args) "0"))
       (seeds (reverse (fold-packages (lambda (p acc) (cons p acc)) '())))
       (progress (lambda (n total) (format (current-error-port) "PROGRESS ~a ~a~%" n total)
                                  (force-output (current-error-port)))))
  (write-index (collect-package-graph seeds progress) commit generated-ms
               (current-output-port) (current-error-port)))
