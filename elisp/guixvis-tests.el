;;; guixvis-tests.el --- Regression checks for Guixvis -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; Run with: emacs -Q --batch -L elisp -l guixvis-tests -f ert-run-tests-batch-and-exit
;; Most requests and processes are mocked; one disposable loopback HTTP
;; fixture exercises Emacs's URL parser.  No check changes a Guix profile.

;;; Code:

(require 'ert)
(require 'guixvis)

(defmacro guixvis-test--with-http (&rest body)
  "Run BODY with captured asynchronous requests in `requests'."
  (declare (indent 0) (debug t))
  `(let ((requests nil)
         (guixvis-web-url "http://127.0.0.1:8787")
         (guixvis-request-timeout 60))
     (cl-letf (((symbol-function 'url-retrieve)
                (lambda (url callback _args silent inhibit-cookies)
                  (should silent)
                  (should inhibit-cookies)
                  (should (equal url-proxy-services '(("no_proxy" . ".*"))))
                  (should (= url-max-redirections 0))
                  (let ((buffer (generate-new-buffer " *guixvis mock HTTP*")))
                    (push (vector url callback buffer) requests)
                    buffer))))
       (unwind-protect (progn ,@body)
         (dolist (request requests)
           (when (buffer-live-p (aref request 2))
             (kill-buffer (aref request 2))))))))

(defun guixvis-test--respond (request code data &optional status)
  "Complete captured REQUEST with HTTP CODE, JSON DATA and STATUS."
  (with-current-buffer (aref request 2)
    (insert "HTTP/1.1 " (number-to-string code) " Status\r\n\r\n")
    (setq-local url-http-response-status code
                url-http-end-of-headers (point-marker))
    (insert (json-encode data))
    (funcall (aref request 1) status)))

(defun guixvis-test--item (name)
  "Return a minimal search item named NAME."
  `((name . ,name) (version . "1.0") (synopsis . "Text editor")
    (deps . 2) (dependents . 12)))

(ert-deftest guixvis-exact-path-keeps-zero ()
  (let ((snapshot (make-string 64 ?a)))
    (should (equal (guixvis--package-path "same" 0 snapshot)
                   (concat "package/same?id=0&snapshot=" snapshot)))
    (should (equal (guixvis--package-path "same" nil nil) "package/same"))
    (dolist (pair `((0 nil) (nil ,snapshot) (-1 ,snapshot)
                   (4294967296 ,snapshot) (0 "bad")))
      (should-error (guixvis--package-path "same" (car pair) (cadr pair)) :type 'user-error))))

(ert-deftest guixvis-exact-search-keeps-same-name-rows-distinct ()
  (with-temp-buffer
    (guixvis-search-mode)
    (let* ((snapshot (make-string 64 ?a))
           (items (mapcar (lambda (id) (append `((id . ,id) (snapshot . ,snapshot))
                                               (guixvis-test--item "same"))) '(0 1))))
      (guixvis--render-search `((items . ,items) (snapshot . ,snapshot)))
      (should (equal (caar tabulated-list-entries) (cons snapshot 0)))
      (should (equal (car (cadr tabulated-list-entries)) (cons snapshot 1))))))

(ert-deftest guixvis-url-allows-only-loopback-origins ()
  (dolist (url '("http://127.0.0.1:8787" "https://localhost:443/"
                 "http://[::1]:8787" "http://localhost"))
    (let ((guixvis-web-url url))
      (should (equal (guixvis--base-url) (string-remove-suffix "/" url)))))
  (dolist (url '("http://example.org:8787" "file:///etc/passwd"
                 "http://localhost.example.org" "http://localhost@evil.org"
                 "http://user:pass@localhost" "http://127.0.0.1/path"
                 "http://127.0.0.1?x=y" "http://127.0.0.1/#fragment"
                 "http://127.0.0.1:0" "http://127.0.0.1:65536"
                 "http://127.0.0.1:\n80" "http://127.0.0.1\\@evil.org"))
    (let ((guixvis-web-url url))
      (should-error (guixvis--base-url) :type 'user-error))))

(ert-deftest guixvis-exact-detail-refresh-links-and-stale-recovery ()
  (guixvis-test--with-http
    (with-temp-buffer
      (guixvis-package-mode)
      (let* ((snapshot (make-string 64 ?a))
             (ref `((name . "same") (id . 1) (snapshot . ,snapshot)))
             (data (append ref '((origin . ((system . "x86_64-linux") (executable . "/fixture/bin/guix") (verified . nil)))
                                (version . "1") (synopsis . "Variant") (command_safe . nil)
                                (catalog . nil)))))
        (setq guixvis--package-name "same" guixvis--package-id 1 guixvis--snapshot snapshot)
        (guixvis--refresh-package)
        (should (string-suffix-p (concat "?id=1&snapshot=" snapshot) (aref (car requests) 0)))
        (guixvis-test--respond (car requests) 200
                              (append data `((deps . (((name . "same") (id . 0) (snapshot . ,snapshot)))))))
        (should (string-match-p "Private or ambiguous" (buffer-string)))
        (should (string-match-p "Guix origin unverified" (buffer-string)))
        (goto-char (point-min))
        (search-forward "Dependencies")
        (forward-button 1)
        (let ((opened nil))
          (cl-letf (((symbol-function 'guixvis--open-package)
                     (lambda (name id token) (setq opened (list name id token)))))
            (push-button)
            (should (equal opened (list "same" 0 snapshot)))))
        (goto-char (point-min))
        (let ((kill-ring nil))
          (guixvis-copy-command "install")
          (should (equal (car kill-ring) "guix install -- 'same@1'")))
        (guixvis--refresh-package)
        (guixvis-test--respond (car requests) 409 nil)
        (should-not guixvis--package-id)
        (should-not guixvis--snapshot)
        (should-not guixvis--package-name)
        (should-not guixvis--package-data)
        (should-error (guixvis--refresh-package) :type 'user-error)
        (should (string-match-p "search again" (buffer-string)))))))

(ert-deftest guixvis-exact-detail-rejects-other-variant-and-old-snapshot ()
  (guixvis-test--with-http
    (with-temp-buffer
      (guixvis-package-mode)
      (setq guixvis--package-name "same" guixvis--package-id 1 guixvis--snapshot (make-string 64 ?a))
      (dolist (ref `(((id . 0) (snapshot . ,guixvis--snapshot))
                    ((id . 1) (snapshot . ,(make-string 64 ?b)))))
        (guixvis--refresh-package)
        (guixvis-test--respond (car requests) 200 (append '((name . "same")) ref))
        (should-not guixvis--package-data)
        (should (string-match-p "Unexpected package identity" (buffer-string)))))))

(ert-deftest guixvis-exact-second-row-opens-second-variant ()
  (with-temp-buffer
    (guixvis-search-mode)
    (let* ((snapshot (make-string 64 ?a))
           (items (mapcar (lambda (id) (append `((id . ,id) (snapshot . ,snapshot))
                                               (guixvis-test--item "same"))) '(0 1)))
           (opened nil))
      (guixvis--render-search `((items . ,items)))
      (goto-char (point-min))
      (forward-line 1)
      (cl-letf (((symbol-function 'guixvis--open-package)
                 (lambda (name id token) (setq opened (list name id token)))))
        (guixvis-package-at-point)
        (should (equal opened (list "same" 1 snapshot)))))))

(ert-deftest guixvis-web-validates-before-opening-browser ()
  (let ((guixvis-web-url "https://example.org") (opened nil))
    (cl-letf (((symbol-function 'browse-url) (lambda (&rest _) (setq opened t))))
      (should-error (guixvis-web) :type 'user-error)
      (should-not opened))))

(ert-deftest guixvis-command-quoting-and-shell-separator ()
  (should (equal (guixvis--command "install" "gtk+") "guix install -- 'gtk+'"))
  (should (equal (guixvis--command "remove" "emacs") "guix remove -- 'emacs'"))
  (should (equal (guixvis--command "show" "emacs") "guix show -- 'emacs'"))
  (should (equal (guixvis--command "shell" "emacs") "guix shell 'emacs'"))
  (dolist (name '("--help" "a'b" "$(id)" "a;id" "a\nb" "" "a b" "é"))
    (should-error (guixvis--command "install" name) :type 'user-error))
  (should-error (guixvis--command "pull" "emacs") :type 'user-error))

(ert-deftest guixvis-copy-command-only-updates-kill-ring ()
  (with-temp-buffer
    (guixvis-package-mode)
    (setq guixvis--package-name "emacs")
    (let ((kill-ring nil))
      (cl-letf (((symbol-function 'start-process)
                 (lambda (&rest _) (ert-fail "Must not start a process")))
                ((symbol-function 'call-process)
                 (lambda (&rest _) (ert-fail "Must not call a process"))))
        (guixvis-copy-command "install")
        (should (equal (car kill-ring) "guix install -- 'emacs'"))))))

(ert-deftest guixvis-terminal-reuses-running-process ()
  (let ((buffer (generate-new-buffer " *guixvis terminal test*"))
        (executions 0))
    (unwind-protect
        (cl-letf (((symbol-function 'get-buffer-create) (lambda (_) buffer))
                  ((symbol-function 'comint-check-proc) (lambda (_) t))
                  ((symbol-function 'term-exec) (lambda (&rest _) (cl-incf executions)))
                  ((symbol-function 'term-char-mode) #'ignore)
                  ((symbol-function 'switch-to-buffer) #'ignore))
          (guixvis--terminal "/opt/bin/guixvis" '("--theme" "nord"))
          (should (= executions 0)))
      (kill-buffer buffer))))

(ert-deftest guixvis-terminal-restarts-dead-process-once ()
  (let ((buffer (generate-new-buffer " *guixvis terminal test*"))
        (executions 0))
    (unwind-protect
        (cl-letf (((symbol-function 'get-buffer-create) (lambda (_) buffer))
                  ((symbol-function 'comint-check-proc) (lambda (_) nil))
                  ((symbol-function 'term-mode) #'ignore)
                  ((symbol-function 'term-exec)
                   (lambda (actual-buffer name program _start args)
                     (should (eq actual-buffer buffer))
                     (should (equal name "guixvis"))
                     (should (equal program "/opt/bin/guixvis"))
                     (should (equal args '("--theme" "nord")))
                     (cl-incf executions)))
                  ((symbol-function 'term-char-mode) #'ignore)
                  ((symbol-function 'switch-to-buffer) #'ignore))
          (guixvis--terminal "/opt/bin/guixvis" '("--theme" "nord"))
          (should (= executions 1)))
      (kill-buffer buffer))))

(ert-deftest guixvis-http-parses-json-and-cleans-response ()
  (guixvis-test--with-http
    (let ((result nil) (request nil))
      (setq request (guixvis--request "search?q=emacs" (lambda (data) (setq result data))
                                      (lambda (text) (ert-fail text))))
      (should (buffer-local-value 'url-max-redirections (aref (car requests) 2)))
      (guixvis-test--respond (car requests) 200 '((items . [])))
      (should (equal result '((items))))
      (should (guixvis--request-state-done request))
      (should-not (buffer-live-p (aref (car requests) 2))))))

(ert-deftest guixvis-http-errors-offer-recovery ()
  (dolist (case '((503 "still building") (404 "not found") (302 "redirected")))
    (guixvis-test--with-http
      (let ((failure nil))
        (guixvis--request "package/emacs" (lambda (_) (ert-fail "Unexpected success"))
                          (lambda (text) (setq failure text)))
        (guixvis-test--respond (car requests) (car case) '((error . "detail")))
        (should (string-match-p (cadr case) failure))))))

(ert-deftest guixvis-http-connection-error-explains-startup ()
  (guixvis-test--with-http
    (let ((failure nil))
      (guixvis--request "search" #'ignore (lambda (text) (setq failure text)))
      (guixvis-test--respond (car requests) 0 nil '(:error (error connection-failed)))
      (should (string-match-p "guixvis web" failure))
      (should (string-match-p "guixvis-web-url" failure)))))

(ert-deftest guixvis-http-timeout-cleans-buffer-and-reports-once ()
  (guixvis-test--with-http
    (let ((timeout nil) (failures nil))
      (cl-letf (((symbol-function 'run-at-time)
                 (lambda (_seconds _repeat function &rest _) (setq timeout function))))
        (guixvis--request "search" #'ignore (lambda (text) (push text failures)))
        (funcall timeout)
        (funcall timeout)
        (should (= (length failures) 1))
        (should (string-match-p "timed out" (car failures)))
        (should-not (buffer-live-p (aref (car requests) 2)))))))

(ert-deftest guixvis-http-rejects-invalid-json ()
  (guixvis-test--with-http
    (let ((failure nil))
      (guixvis--request "search" #'ignore (lambda (text) (setq failure text)))
      (with-current-buffer (aref (car requests) 2)
        (insert "HTTP/1.1 200 OK\r\n\r\n")
        (setq-local url-http-response-status 200
                    url-http-end-of-headers (point-marker))
        (insert "{broken")
        (funcall (aref (car requests) 1) nil))
      (should (string-match-p "invalid JSON" failure)))))

(ert-deftest guixvis-search-uses-api-and-renders-rows ()
  (guixvis-test--with-http
    (with-temp-buffer
      (guixvis-search-mode)
      (setq guixvis--query "text éditor")
      (guixvis--refresh-search)
      (should (equal "http://127.0.0.1:8787/api/v1/search?q=text%20%C3%A9ditor&limit=100"
                     (aref (car requests) 0)))
      (guixvis-test--respond (car requests) 200
                            `((items . [,(guixvis-test--item "emacs")]) (capped . t)))
      (should (equal (caar tabulated-list-entries) "emacs"))
      (should (equal " [1+ packages]" mode-line-process))
      (should (listp header-line-format))
      (goto-char (point-min))
      (should (equal (tabulated-list-get-id) "emacs")))))

(ert-deftest guixvis-new-search-discards-older-response ()
  (with-temp-buffer
    (guixvis-search-mode)
    (let ((callbacks nil))
      (cl-letf (((symbol-function 'guixvis--request)
                 (lambda (_path success failure)
                   (push (cons success failure) callbacks)
                   nil)))
        (guixvis--fetch "search?q=old" #'guixvis--render-search)
        (guixvis--fetch "search?q=new" #'guixvis--render-search)
        (funcall (caar callbacks) `((items . (,(guixvis-test--item "new")))))
        (funcall (car (cadr callbacks)) `((items . (,(guixvis-test--item "old")))))
        (funcall (cdr (cadr callbacks)) "Old error")
        (should (equal (caar tabulated-list-entries) "new"))
        (should-not (equal header-line-format "Old error"))))))

(ert-deftest guixvis-killed-buffer-discards-response ()
  (let ((buffer (generate-new-buffer " *guixvis closed*"))
        (success nil) (failure nil) (rendered nil))
    (cl-letf (((symbol-function 'guixvis--request)
               (lambda (_path on-success on-failure)
                 (setq success on-success failure on-failure)
                 nil)))
      (with-current-buffer buffer
        (guixvis-search-mode)
        (guixvis--fetch "search" (lambda (_) (setq rendered t))))
      (kill-buffer buffer)
      (funcall success '((items)))
      (funcall failure "Disconnected")
      (should-not rendered))))

(ert-deftest guixvis-invalid-search-data-is-rejected ()
  (with-temp-buffer
    (guixvis-search-mode)
    (should-error (guixvis--render-search '((not-items . []))))
    (should-error (guixvis--search-entry '((name . "--help") (deps . 0) (dependents . 0))))
    (should-error (guixvis--search-entry '((name . "emacs") (deps . "0") (dependents . 0))))))

(ert-deftest guixvis-details-render-clickable-dependencies ()
  (with-temp-buffer
    (guixvis-package-mode)
    (setq guixvis--package-name "emacs")
    (guixvis--render-package
     '((name . "emacs") (version . "30") (synopsis . "Editor")
       (description . "An extensible editor.") (licenses . ("GPL-3.0+"))
       (homepage . "https://gnu.org/software/emacs/") (file . "gnu/packages/emacs.scm")
       (line . 42) (deps . (((name . "gtk+") (kind . "input"))))
       (dependents . nil) (module_neighbors . nil)))
    (should (string-match-p "GPL-3.0+" (buffer-string)))
    (should (string-match-p "emacs.scm:42" (buffer-string)))
    (goto-char (point-min))
    (search-forward "gtk+")
    (backward-char)
    (should (equal (guixvis--name-at-point) "gtk+"))
    (let ((selected nil))
      (cl-letf (((symbol-function 'guixvis-package) (lambda (name) (setq selected name))))
        (push-button)
        (should (equal selected "gtk+"))))))

(ert-deftest guixvis-details-reject-wrong-package ()
  (with-temp-buffer
    (guixvis-package-mode)
    (setq guixvis--package-name "emacs")
    (should-error (guixvis--render-package '((name . "vim"))))))

(ert-deftest guixvis-search-limits-are-validated ()
  (with-temp-buffer
    (guixvis-search-mode)
    (dolist (guixvis-search-limit '(0 501 "100"))
      (should-error (guixvis--refresh-search) :type 'user-error))
    (should-error (guixvis-search (make-string 201 ?a)) :type 'user-error)))

(ert-deftest guixvis-live-loopback-http-and-redirect-policy ()
  ;; Exercise Emacs's real URL parser against a disposable local fixture.
  (let ((server nil) (connections nil) (request-count 0) (pending nil))
    (unwind-protect
        (progn
          (setq server
                (make-network-process
                 :name "guixvis-http-fixture" :server t :host "127.0.0.1"
                 :service t :noquery t :coding 'binary
                 :log (lambda (_server client _message) (push client connections))
                 :filter
                 (lambda (client chunk)
                   (let ((input (concat (process-get client 'input) chunk)))
                     (process-put client 'input input)
                     (when (and (string-match-p "\r\n\r\n" input)
                                (not (process-get client 'sent)))
                       (process-put client 'sent t)
                       (cl-incf request-count)
                       (if (string-match-p "GET /api/v1/redirect " input)
                           (process-send-string
                            client
                            (format (concat "HTTP/1.1 302 Found\r\nLocation: "
                                            "http://127.0.0.1:%d/should-not-follow\r\n"
                                            "Content-Length: 0\r\nConnection: close\r\n\r\n")
                                    (process-contact server :service)))
                         (let ((body (encode-coding-string "{\"message\":\"Olá\"}" 'utf-8)))
                           (process-send-string
                            client
                            (concat (format (concat "HTTP/1.1 200 OK\r\n"
                                                    "Content-Type: application/json\r\n"
                                                    "Content-Length: %d\r\n"
                                                    "Connection: close\r\n\r\n")
                                            (string-bytes body)) body))))
                       (process-send-eof client))))))
          (let ((guixvis-web-url (format "http://127.0.0.1:%d"
                                         (process-contact server :service)))
                (guixvis-request-timeout 3)
                (result nil) (failure nil) (finished nil))
            (setq pending (guixvis--request
                           "health" (lambda (data) (setq result data finished t))
                           (lambda (text) (setq failure text finished t))))
            (let ((deadline (+ (float-time) 4)))
              (while (and (not finished) (< (float-time) deadline))
                (accept-process-output nil 0.01)))
            (should finished)
            (should-not failure)
            (should (equal (alist-get 'message result) "Olá"))
            (setq finished nil result nil)
            (setq pending (guixvis--request
                           "redirect" (lambda (data) (setq result data finished t))
                           (lambda (text) (setq failure text finished t))))
            (let ((deadline (+ (float-time) 4)))
              (while (and (not finished) (< (float-time) deadline))
                (accept-process-output nil 0.01)))
            (should finished)
            (should-not result)
            (should (string-match-p "redirected" failure))
            (should (= request-count 2))))
      (guixvis--dispose-request pending)
      (dolist (client connections)
        (when (process-live-p client) (delete-process client)))
      (when (process-live-p server) (delete-process server)))))

(provide 'guixvis-tests)
;;; guixvis-tests.el ends here
