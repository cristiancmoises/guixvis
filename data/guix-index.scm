;;; guixvis — GNU Guix package index generator.
;;;
;;; Usage: guix repl -- guix-index.scm [GUIX_COMMIT] [GENERATED_MS]
;;;
;;; Emits exactly ONE JSON document on stdout:
;;;   {"header":{...},"packages":[ {...}, {...}, ... ]}
;;; Emits progress lines on stderr:
;;;   PROGRESS <processed> <total>
;;;
;;; Pure and read-only: never mutates the store, never touches the network.
;;; Memory stays bounded (JSON is streamed per package to an explicit port).

(use-modules (guix packages)
             (guix diagnostics)
             (guix licenses)
             (gnu packages)
             (json)
             (srfi srfi-1))

(define commit  (if (and (pair? (command-line)) (pair? (cdr (command-line))))
                    (cadr (command-line))
                    ""))
(define gen-ms  (let ((a (cdr (command-line))))
                  (if (and (pair? a) (pair? (cdr a))) (cadr a) "0")))

;; ---------------------------------------------------------------------------
;; Defensive accessors: a malformed package or input object must never abort
;; the whole walk.
;; ---------------------------------------------------------------------------

(define (str-safe thunk)
  (catch #t
    (lambda () (let ((s (thunk))) (if (string? s) s "")))
    (lambda _ "")))

(define (package-name-safe p)
  (catch #t
    (lambda () (let ((n (package-name p))) (and (string? n) n)))
    (lambda _ #f)))

(define (dep-name-of x)
  ;; Walk any input shape: bare package, ("label" . pkg) dotted pair,
  ;; ("label" pkg) proper pair, (pkg1 pkg2 ...) lists, nested wrappers.
  ;; Origins, file-append, module-ref and other structs are skipped.
  ;; Depth bound + seen-pair set guard against improper or cyclic lists.
  (let walk ((x x) (depth 0) (seen '()))
    (and (< depth 64)
         (cond
          ((package? x) (package-name-safe x))
          ((pair? x)
           (and (not (memq x seen))
                (or (walk (car x) (+ depth 1) (cons x seen))
                    (walk (cdr x) (+ depth 1) (cons x seen)))))
          (else #f)))))

(define (dep-names-of accessor p)
  (catch #t
    (lambda ()
      (delete-duplicates
       (filter-map dep-name-of (accessor p))
       string=?))
    (lambda _ '())))

(define (licenses-of p)
  (catch #t
    (lambda ()
      (let ((l (package-license p)))
        (cond
         ((license? l)
          (let ((n (license-name l))) (if (string? n) (list n) '())))
         ((list? l)
          (filter-map (lambda (x)
                        (and (license? x)
                             (let ((n (license-name x)))
                               (and (string? n) n))))
                      l))
         (else '()))))
    (lambda _ '())))

(define (location-of p)
  (catch #t
    (lambda ()
      (let ((l (package-location p)))
        (if l
            (let ((f (location-file l)) (ln (location-line l)))
              (list (if (string? f) f "")
                    (if (number? ln) ln 0)))
            (list "" 0))))
    (lambda _ (list "" 0))))

(define (pkg->alist id p)
  (list
   (cons "id" id)
   (cons "name" (str-safe (lambda () (package-name p))))
   (cons "version" (str-safe (lambda () (package-version p))))
   (cons "synopsis" (str-safe (lambda () (package-synopsis p))))
   (cons "description" (str-safe (lambda () (package-description p))))
   (cons "homepage" (str-safe (lambda () (package-home-page p))))
   (cons "licenses" (list->vector (licenses-of p)))
   (cons "file" (list->vector (location-of p))) ; ["gnu/packages/x.scm", line]
   (cons "inputs" (list->vector (dep-names-of package-inputs p)))
   (cons "propagated_inputs" (list->vector (dep-names-of package-propagated-inputs p)))
   (cons "native_inputs" (list->vector (dep-names-of package-native-inputs p)))))

;; ---------------------------------------------------------------------------
;; Two-pass walk: assign ids, then stream JSON per package.
;; ---------------------------------------------------------------------------

(define (main)
  (let* ((id->pkg (make-hash-table 40000))
         (total 0))
    (fold-packages
     (lambda (p _)
       (hash-set! id->pkg total p)
       (set! total (+ total 1))
       #f)
     #f)

    (let ((out (current-output-port))
          (err (current-error-port)))
      (display "{\"header\":" out)
      (scm->json `(("schema" . 3)
                   ("guix_commit" . ,commit)
                   ("generated_ms" . ,gen-ms)
                   ("package_count" . ,total))
                 out)
      (display ",\"packages\":[" out)

      (let loop ((id 0))
        (cond
         ((>= id total) (display "]}\n" out))
         (else
          (scm->json (pkg->alist id (hash-ref id->pkg id)) out)
          (unless (= id (- total 1)) (display "," out))
          (when (or (zero? (modulo (+ id 1) 500)) (= id (- total 1)))
            (format err "PROGRESS ~a ~a~%" (+ id 1) total)
            (force-output err))
          (loop (+ id 1)))))

      (force-output out)
      (force-output err))))

(main)
