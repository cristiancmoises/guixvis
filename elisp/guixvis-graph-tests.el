;;; guixvis-graph-tests.el --- Native graph checks -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; Run with the core suite, which supplies the disposable HTTP fixture.

;;; Code:

(require 'guixvis-tests)
(require 'guixvis-graph nil t)

(defun guixvis-graph-test--data (&optional root direction depth)
  "Return a cyclic graph with shared dependencies and same-name variants.
ROOT defaults to zero, DIRECTION to deps, and DEPTH to two."
  (let* ((root (or root 0)) (depth (or depth 2))
         (snapshot (make-string 64 ?a))
         (nodes (cl-loop for (id name version catalog) in
                         '((0 "alpha" "1" t) (1 "beta" "1" t)
                           (2 "beta" "2" nil) (3 "gamma" "3" t))
                         collect `((id . ,id) (name . ,name)
                                   (version . ,version) (catalog . ,catalog)
                                   (snapshot . ,snapshot) (degree . 3)
                                   (depth . ,(if (= id root) 0 (min depth (if (= id 3) 2 1))))
                                   (kinds . ("input")) (kind . "input"))))
         (edges (cl-loop for (from to) in '((0 1) (0 2) (1 3) (2 3) (3 0))
                         collect `((from_id . ,from) (to_id . ,to)
                                   (from . ,(alist-get 'name (nth from nodes)))
                                   (to . ,(alist-get 'name (nth to nodes)))
                                   (kinds . ("input"))))))
    (copy-tree `((snapshot . ,snapshot) (root_id . ,root)
      (root . ,(alist-get 'name (nth root nodes)))
      (dir . ,(or direction "deps")) (depth . ,depth)
      (complete . t) (diagnostics_count . 0) (materialized . 4)
      (truncated . 0) (discovered_total . 4) (discovery_complete . t)
      (edges_total . 5) (edges_truncated . 0) (nodes . ,nodes) (edges . ,edges)))))

(defmacro guixvis-graph-test--buffer (&rest body)
  "Run BODY in a graph buffer rooted at alpha, depth two."
  (declare (indent 0) (debug t))
  `(with-temp-buffer
     (guixvis-graph-mode)
     (setq guixvis--package-name "alpha" guixvis--graph-depth 2)
     ,@body))

(defun guixvis-graph-test--select (id)
  "Move to a graph button bearing ID."
  (goto-char (point-min))
  (let ((button (next-button (point-min))))
    (while (and button (not (equal id (alist-get 'id (button-get button 'guixvis-ref)))))
      (setq button (next-button (button-end button))))
    (should button)
    (goto-char (button-start button))))

(ert-deftest guixvis-graph-shows-cycles-shared-variants-and-exact-buttons ()
  (guixvis-graph-test--buffer
    (guixvis--render-graph (guixvis-graph-test--data))
    (should (equal guixvis--package-id 0))
    (should (equal guixvis--snapshot (make-string 64 ?a)))
    (should (string-match-p "Depth 2" (buffer-string)))
    (should (string-match-p "Adjacency" (buffer-string)))
    (should (string-match-p "gamma@3 · ID 3.*→.*alpha@1 · ID 0" (buffer-string)))
    (should (string-match-p "beta@1 · ID 1.*→.*gamma@3 · ID 3" (buffer-string)))
    (should (string-match-p "beta@2 · ID 2.*→.*gamma@3 · ID 3" (buffer-string)))
    (should (string-match-p "private" (buffer-string)))
    (guixvis-graph-test--select 2)
    (should (equal (alist-get 'version (guixvis--ref-at-point)) "2"))
    (should (equal (alist-get 'snapshot (guixvis--ref-at-point)) (make-string 64 ?a)))))

(ert-deftest guixvis-graph-validates-response-atomically ()
  ;; Every mutation must reject the whole projection before a button exists.
  (dolist (mutation
           (list (lambda (d) (setf (alist-get 'root d) "other"))
                 (lambda (d) (setf (alist-get 'root_id d) 4294967296))
                 (lambda (d) (setf (alist-get 'snapshot d) "bad"))
                 (lambda (d) (setf (alist-get 'dir d) "reverse"))
                 (lambda (d) (setf (alist-get 'depth d) 9))
                 (lambda (d) (setf (alist-get 'materialized d) 3))
                 (lambda (d) (setf (alist-get 'truncated d) -1))
                 (lambda (d) (setf (alist-get 'discovered_total d) 5))
                 (lambda (d) (setf (alist-get 'discovery_complete d) "true"))
                 (lambda (d) (setf (alist-get 'edges_total d) 4))
                 (lambda (d) (setf (alist-get 'edges_truncated d) -1))
                 (lambda (d) (setf (alist-get 'complete d) 1))
                 (lambda (d) (setf (alist-get 'diagnostics_count d) 1))
                 (lambda (d) (setf (alist-get 'id (nth 1 (alist-get 'nodes d))) 0))
                 (lambda (d) (setf (alist-get 'id (nth 1 (alist-get 'nodes d))) -1))
                 (lambda (d) (setf (alist-get 'name (nth 1 (alist-get 'nodes d))) "bad/name"))
                 (lambda (d) (setf (alist-get 'snapshot (nth 1 (alist-get 'nodes d))) (make-string 64 ?b)))
                 (lambda (d) (setf (alist-get 'depth (nth 0 (alist-get 'nodes d))) 1))
                 (lambda (d) (setf (alist-get 'depth (nth 1 (alist-get 'nodes d))) 0))
                 (lambda (d) (setf (alist-get 'depth (nth 1 (alist-get 'nodes d))) 3))
                 (lambda (d) (setf (alist-get 'catalog (nth 1 (alist-get 'nodes d))) "yes"))
                 (lambda (d) (setf (alist-get 'degree (nth 1 (alist-get 'nodes d))) -1))
                 (lambda (d) (setf (alist-get 'version (nth 1 (alist-get 'nodes d))) 1))
                 (lambda (d) (setf (alist-get 'kinds (nth 1 (alist-get 'nodes d))) '("unknown")))
                 (lambda (d) (setf (alist-get 'kind (nth 1 (alist-get 'nodes d))) "unknown"))
                 (lambda (d) (setf (alist-get 'to_id (car (alist-get 'edges d))) 99))
                 (lambda (d) (setf (alist-get 'to (car (alist-get 'edges d))) "gamma"))
                 (lambda (d) (setf (alist-get 'kinds (car (alist-get 'edges d))) nil))
                 (lambda (d) (push (copy-tree (car (alist-get 'edges d))) (alist-get 'edges d))
                   (setf (alist-get 'edges_total d) 6))))
    (guixvis-graph-test--buffer
      (let ((data (guixvis-graph-test--data)))
        (funcall mutation data)
        (should-error (guixvis--render-graph data))
        (should-not guixvis--graph-data)
        (should-not (next-button (point-min)))
        (should-not guixvis--package-id)))))

(ert-deftest guixvis-graph-rejects-pinned-variant-and-snapshot-mismatch ()
  (guixvis-graph-test--buffer
    (setq guixvis--package-id 0 guixvis--snapshot (make-string 64 ?a))
    (let ((data (guixvis-graph-test--data)))
      (setf (alist-get 'root_id data) 2)
      (should-error (guixvis--render-graph data)))
    (let ((data (guixvis-graph-test--data)))
      (setf (alist-get 'snapshot data) (make-string 64 ?b))
      (should-error (guixvis--render-graph data)))))

(ert-deftest guixvis-graph-rejects-node-and-edge-budget-overflow ()
  (guixvis-graph-test--buffer
    (let ((data (guixvis-graph-test--data)))
      (setf (alist-get 'nodes data) (make-list 201 (car (alist-get 'nodes data)))
            (alist-get 'materialized data) 201 (alist-get 'discovered_total data) 201)
      (should-error (guixvis--render-graph data)))
    (let ((data (guixvis-graph-test--data)))
      (setf (alist-get 'edges data) (make-list 3001 (car (alist-get 'edges data)))
            (alist-get 'edges_total data) 3001)
      (should-error (guixvis--render-graph data)))))

(ert-deftest guixvis-graph-filter-is-literal-local-and-retains-root-context ()
  (guixvis-graph-test--buffer
    (guixvis--render-graph (guixvis-graph-test--data))
    (guixvis-graph-test--select 2)
    (guixvis-graph-filter "BETA")
    (should (string-match-p "Root: alpha@1 · ID 0" (buffer-string)))
    (should (string-match-p "2/4 nodes" (buffer-string)))
    (should-not (string-match-p "gamma@3" (buffer-string)))
    (should (equal (alist-get 'id (guixvis--ref-at-point)) 2))
    (guixvis-graph-filter ".*")
    (should (string-match-p "0/4 nodes" (buffer-string)))
    (should-not (string-match-p "Depth 1" (buffer-string)))
    (guixvis-graph-filter "")
    (should (string-match-p "4/4 nodes" (buffer-string)))))

(ert-deftest guixvis-graph-reports-unknown-and-incomplete-counts ()
  (guixvis-graph-test--buffer
    (let ((data (guixvis-graph-test--data)))
      (setf (alist-get 'discovered_total data) nil
            (alist-get 'discovery_complete data) nil
            (alist-get 'truncated data) 0
            (alist-get 'edges_total data) nil
            (alist-get 'edges_truncated data) 0
            (alist-get 'complete data) nil (alist-get 'diagnostics_count data) 3)
      (guixvis--render-graph data)
      (should (string-match-p "discovered unknown" (buffer-string)))
      (should (string-match-p "at least 0" (buffer-string)))
      (should (string-match-p "total unknown" (buffer-string)))
      (should (string-match-p "3 extraction diagnostics" (buffer-string))))))

(ert-deftest guixvis-graph-requests-follow-back-depth-direction-and-selection ()
  (guixvis-test--with-http
    (guixvis-graph-test--buffer
      (guixvis--refresh-graph)
      (should (string-suffix-p "graph/alpha?dir=deps&depth=2&budget=200" (aref (car requests) 0)))
      (guixvis-test--respond (car requests) 200 (guixvis-graph-test--data))
      (guixvis-graph-test--select 2)
      (guixvis-graph-filter "beta")
      (guixvis-graph-follow)
      (should (equal (aref (car requests) 0)
                     (concat "http://127.0.0.1:8787/api/v1/graph/beta?id=2&snapshot="
                             (make-string 64 ?a) "&dir=deps&depth=2&budget=200")))
      (guixvis-test--respond (car requests) 200 (guixvis-graph-test--data 2))
      (should (equal guixvis--graph-filter ""))
      (guixvis-graph-back)
      (guixvis-test--respond (car requests) 200 (guixvis-graph-test--data))
      (should (equal guixvis--graph-filter "beta"))
      (should (equal (alist-get 'id (guixvis--ref-at-point)) 2))
      (guixvis-graph-toggle-direction)
      (should (string-suffix-p "&dir=reverse&depth=2&budget=200" (aref (car requests) 0)))
      (guixvis-test--respond (car requests) 200 (guixvis-graph-test--data 0 "reverse"))
      (guixvis-graph-increase-depth)
      (should (string-suffix-p "&dir=reverse&depth=3&budget=200" (aref (car requests) 0)))
      (guixvis-test--respond (car requests) 200 (guixvis-graph-test--data 0 "reverse" 3))
      (guixvis-graph-decrease-depth)
      (should (string-suffix-p "&dir=reverse&depth=2&budget=200" (aref (car requests) 0))))))

(ert-deftest guixvis-graph-history-is-bounded-and-depth-stops-at-limits ()
  (guixvis-test--with-http
    (guixvis-graph-test--buffer
      (guixvis--render-graph (guixvis-graph-test--data))
      (dotimes (_ 35)
        (guixvis-graph-test--select 2)
        (guixvis-graph-follow)
        (guixvis-test--respond (car requests) 200 (guixvis-graph-test--data 2)))
      (should (= (length guixvis--graph-history) 32))
      (setq guixvis--graph-depth 8)
      (let ((count (length requests)))
        (guixvis-graph-increase-depth)
        (should (= count (length requests))))
      (setq guixvis--graph-depth 1)
      (let ((count (length requests)))
        (guixvis-graph-decrease-depth)
        (should (= count (length requests))))
      (setq guixvis--graph-history nil)
      (should-not (guixvis-graph-back)))))

(ert-deftest guixvis-graph-errors-remove-actions-history-and-ignore-late-responses ()
  (guixvis-test--with-http
    (guixvis-graph-test--buffer
      (guixvis--refresh-graph)
      (let ((old (car requests)))
        (guixvis--refresh-graph)
        (guixvis-test--respond (car requests) 200 (guixvis-graph-test--data))
        (should guixvis--graph-data)
        ;; The shared fetch cancellation must have killed this older buffer.
        (should-not (buffer-live-p (aref old 2))))
      (setq guixvis--graph-history '(context))
      (guixvis--refresh-graph)
      (let ((bad (guixvis-graph-test--data)))
        (setf (alist-get 'to_id (car (alist-get 'edges bad))) 99)
        (guixvis-test--respond (car requests) 200 bad))
      (should-not guixvis--graph-data)
      (should-not guixvis--graph-history)
      (should-not (next-button (point-min)))
      (should-error (guixvis-graph-follow) :type 'user-error)
      (should-error (guixvis-graph-copy-command) :type 'user-error)
      (should-error (guixvis-package-at-point) :type 'user-error)
      (guixvis--refresh-graph)
      (guixvis-test--respond (car requests) 409 nil)
      (should-not guixvis--package-name)
      (should-not guixvis--package-id)
      (should-not guixvis--snapshot)
      (should-error (guixvis--refresh-graph) :type 'user-error)
      (should (string-match-p "search again" (buffer-string))))))

(ert-deftest guixvis-graph-selected-details-and-show-command-preserve-variant ()
  (guixvis-graph-test--buffer
    (guixvis--render-graph (guixvis-graph-test--data))
    (guixvis-graph-test--select 2)
    (let ((opened nil) (kill-ring nil))
      (cl-letf (((symbol-function 'guixvis--open-package)
                 (lambda (name id snapshot) (setq opened (list name id snapshot)))))
        (guixvis-package-at-point)
        (should (equal opened (list "beta" 2 (make-string 64 ?a)))))
      (guixvis-graph-copy-command)
      (should (equal (car kill-ring) "guix show -- 'beta@2'")))))

(ert-deftest guixvis-graph-button-activation-follows-its-own-identity ()
  (guixvis-test--with-http
    (guixvis-graph-test--buffer
      (guixvis--render-graph (guixvis-graph-test--data))
      (guixvis-graph-test--select 2)
      (let ((selected (button-at (point))))
        (guixvis-graph-test--select 1)
        (button-activate selected)
        (should (string-prefix-p
                 "http://127.0.0.1:8787/api/v1/graph/beta?id=2&"
                 (aref (car requests) 0)))))))

(ert-deftest guixvis-graph-top-level-malformation-does-not-echo-payload ()
  (dolist (data '("UNTRUSTED-PAYLOAD" 42 t ((nodes . 42))))
    (guixvis-test--with-http
      (guixvis-graph-test--buffer
        (guixvis--refresh-graph)
        (guixvis-test--respond (car requests) 200 data)
        (should-not guixvis--graph-data)
        (should-not (string-match-p "UNTRUSTED-PAYLOAD" (buffer-string)))
        (should (string-match-p "Invalid graph response" (buffer-string)))))))

(ert-deftest guixvis-graph-truncation-requires-full-materialization-budget ()
  (guixvis-graph-test--buffer
    (let ((data (guixvis-graph-test--data)))
      (setf (alist-get 'discovered_total data) 5 (alist-get 'truncated data) 1)
      (should-error (guixvis--render-graph data)))
    (let ((data (guixvis-graph-test--data)))
      (setf (alist-get 'edges_total data) 6 (alist-get 'edges_truncated data) 1)
      (should-error (guixvis--render-graph data)))))

(ert-deftest guixvis-graph-bounds-version-work ()
  (guixvis-graph-test--buffer
    (let ((data (guixvis-graph-test--data)))
      (setf (alist-get 'version (nth 1 (alist-get 'nodes data))) (make-string 513 ?x))
      (should-error (guixvis--render-graph data))
      (setf (alist-get 'version (nth 1 (alist-get 'nodes data))) (make-string 512 ?x))
      (guixvis--render-graph data))))

(ert-deftest guixvis-graph-bounds-filter-work ()
  (guixvis-graph-test--buffer
    (guixvis--render-graph (guixvis-graph-test--data))
    (let ((before (buffer-string)))
      (should-error (guixvis-graph-filter (make-string 201 ?x)) :type 'user-error)
      (should (equal (buffer-string) before))
      (should (equal guixvis--graph-filter "")))
    (guixvis-graph-filter (make-string 200 ?x))
    (should (string-match-p "0/4 nodes" (buffer-string)))))

(ert-deftest guixvis-graph-allows-independent-edge-scan-and-optional-legacy-kind ()
  (guixvis-graph-test--buffer
    (let ((data (guixvis-graph-test--data)))
      (setf (alist-get 'edges_total data) nil)
      (dolist (node (alist-get 'nodes data))
        (setf (alist-get 'kind node) nil))
      (setf (alist-get 'version (nth 1 (alist-get 'nodes data))) "1.Ω\n2")
      (guixvis--render-graph data)
      (should (string-match-p "total unknown" (buffer-string)))
      (should (string-match-p "beta@1.Ω 2" (buffer-string)))
      (guixvis-graph-test--select 1)
      (should-error (guixvis-graph-copy-command) :type 'user-error))))

(ert-deftest guixvis-graph-full-budget-preserves-known-and-unknown-truncation ()
  (guixvis-graph-test--buffer
    (let* ((data (guixvis-graph-test--data))
           (base (car (alist-get 'nodes data)))
           (nodes (cl-loop for id below 200 collect
                           (let ((node (copy-tree base)))
                             (setf (alist-get 'id node) id
                                   (alist-get 'depth node) (if (= id 0) 0 1))
                             node)))
           (edges (cl-loop for from below 15 append
                           (cl-loop for to below 200 collect
                                    `((from_id . ,from) (to_id . ,to)
                                      (from . "alpha") (to . "alpha")
                                      (kinds . ("input")))))))
      (setf (alist-get 'nodes data) nodes (alist-get 'edges data) edges
            (alist-get 'materialized data) 200 (alist-get 'discovered_total data) 250
            (alist-get 'truncated data) 50 (alist-get 'edges_total data) 3020
            (alist-get 'edges_truncated data) 20)
      (guixvis--render-graph data)
      (should (string-match-p "materialized 200 · discovered 250 · truncated 50" (buffer-string)))
      (should (string-match-p "materialized 3000 · total 3020 · truncated 20" (buffer-string)))
      (setf (alist-get 'discovered_total data) nil (alist-get 'discovery_complete data) nil
            (alist-get 'edges_total data) nil)
      (guixvis--render-graph data)
      (should (string-match-p "at least 50" (buffer-string)))
      (should (string-match-p "at least 20" (buffer-string))))))

(ert-deftest guixvis-graph-cleanup-on-kill-and-major-mode-change ()
  (guixvis-test--with-http
    (let ((target (generate-new-buffer " *graph lifecycle*")))
      (unwind-protect
          (progn
            (with-current-buffer target
              (guixvis-graph-mode)
              (setq guixvis--package-name "alpha")
              (guixvis--refresh-graph)
              (special-mode)
              (should-not guixvis--request-token))
            (should-not (buffer-live-p (aref (car requests) 2)))
            (with-current-buffer target
              (guixvis-graph-mode)
              (setq guixvis--package-name "alpha")
              (guixvis--refresh-graph))
            (kill-buffer target)
            (should-not (buffer-live-p (aref (car requests) 2))))
        (when (buffer-live-p target) (kill-buffer target))))))

(ert-deftest guixvis-graph-late-callback-cannot-revive-stale-projection ()
  (guixvis-test--with-http
    (guixvis-graph-test--buffer
      (guixvis--refresh-graph)
      (let* ((old (car requests)) (callback (aref old 1)))
        (guixvis--refresh-graph)
        (guixvis-test--respond (car requests) 409 nil)
        ;; A disposed transport may still invoke its callback in another buffer.
        (with-temp-buffer (funcall callback nil))
        (should-not guixvis--graph-data)
        (should-not guixvis--graph-root)
        (should-not (guixvis--ref-at-point))
        (should (string-match-p "search again" (buffer-string)))))))

(ert-deftest guixvis-graph-core-keys-open-native-exact-reference ()
  (dolist (mode '(guixvis-search-mode guixvis-package-mode))
    (with-temp-buffer
      (funcall mode)
      (should (eq (key-binding (kbd "v")) #'guixvis-graph-at-point))))
  (guixvis-graph-test--buffer
    (guixvis--render-graph (guixvis-graph-test--data))
    (dolist (binding '(("RET" . guixvis-graph-follow) ("p" . guixvis-package-at-point)
                       ("/" . guixvis-graph-filter) ("d" . guixvis-graph-toggle-direction)
                       ("+" . guixvis-graph-increase-depth) ("-" . guixvis-graph-decrease-depth)
                       ("l" . guixvis-graph-back) ("w" . guixvis-graph-copy-command)))
      (should (eq (lookup-key (current-local-map) (kbd (car binding))) (cdr binding))))))

(ert-deftest guixvis-graph-opens-exact-search-reference-with-safe-defaults ()
  (guixvis-test--with-http
    (unwind-protect
        (with-temp-buffer
          (guixvis-search-mode)
          (let ((item (append `((id . 2) (snapshot . ,(make-string 64 ?a)))
                              (guixvis-test--item "beta"))))
            (guixvis--render-search `((items . (,item))))
            (goto-char (point-min))
            (guixvis-graph-at-point))
          (with-current-buffer "*Guixvis graph*"
            (should (derived-mode-p 'guixvis-graph-mode))
            (should (equal guixvis--graph-filter ""))
            (should-not guixvis--graph-history)
            (should (string-suffix-p "&dir=deps&depth=1&budget=200" (aref (car requests) 0)))
            (should (string-match-p "id=2&snapshot=" (aref (car requests) 0)))
            (guixvis-test--respond (car requests) 200 (guixvis-graph-test--data 2 "deps" 1))
            (guixvis-graph-test--select 1)
            (guixvis--refresh-graph)
            (guixvis-test--respond (car requests) 200 (guixvis-graph-test--data 2 "deps" 1))
            (should (equal (alist-get 'id (guixvis--ref-at-point)) 1))))
      (when-let* ((buffer (get-buffer "*Guixvis graph*"))) (kill-buffer buffer)))))

(ert-deftest guixvis-graph-http-failure-clears-actions-and-retains-exact-retry ()
  (dolist (code '(404 500 503))
    (guixvis-test--with-http
      (guixvis-graph-test--buffer
        (guixvis--render-graph (guixvis-graph-test--data))
        (setq guixvis--graph-history '(old))
        (guixvis--refresh-graph)
        (should-not (guixvis--ref-at-point))
        (guixvis-test--respond (car requests) code nil)
        (should-not guixvis--graph-data)
        (should-not guixvis--graph-history)
        (should-not (guixvis--ref-at-point))
        (should-not (next-button (point-min)))
        (should-error (guixvis-graph-copy-command) :type 'user-error)
        (guixvis--refresh-graph)
        (should (string-match-p "id=0&snapshot=" (aref (car requests) 0)))
        (guixvis-test--respond (car requests) 200 (guixvis-graph-test--data))
        (should guixvis--graph-data)))))

(provide 'guixvis-graph-tests)
;;; guixvis-graph-tests.el ends here
