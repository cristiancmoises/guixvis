;;; guixvis-graph.el --- Native Guix dependency graphs -*- lexical-binding: t; -*-

;; Copyright © 2026 Cristian Cezar Moisés <cristiancmoises@users.noreply.github.com>
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Version: 0.10.0
;; Package-Requires: ((emacs "27.1"))

;;; Commentary:
;; Browse bounded graph projections from the local Guixvis service.  Depth
;; groups and actual directed adjacency preserve cycles and shared nodes.
;; No external renderer is needed, and no command is executed.  Projections
;; accept at most 200 nodes, 3000 edges, and 512 characters per version;
;; literal local filters are limited to 200 characters.

;;; Code:

(require 'guixvis)

(defvar-local guixvis--graph-data nil)
(defvar-local guixvis--graph-root nil)
(defvar-local guixvis--graph-direction "deps")
(defvar-local guixvis--graph-depth 1)
(defvar-local guixvis--graph-filter "")
(defvar-local guixvis--graph-history nil)
(defvar-local guixvis--graph-selected-id nil)

(defvar guixvis-graph-mode-map
  (let ((map (make-sparse-keymap)))
    (set-keymap-parent map special-mode-map)
    (dolist (binding '(("RET" . guixvis-graph-follow)
                       ("p" . guixvis-package-at-point) ("s" . guixvis-search)
                       ("/" . guixvis-graph-filter) ("g" . guixvis--refresh-graph)
                       ("d" . guixvis-graph-toggle-direction)
                       ("+" . guixvis-graph-increase-depth)
                       ("-" . guixvis-graph-decrease-depth)
                       ("l" . guixvis-graph-back) ("w" . guixvis-graph-copy-command)
                       ("TAB" . forward-button) ("<backtab>" . backward-button)))
      (define-key map (kbd (car binding)) (cdr binding)))
    map)
  "Keymap for native Guixvis graphs.")

(define-derived-mode guixvis-graph-mode special-mode "Guixvis Graph"
  "Navigate graph nodes and their actual dependency edges.
RET follows a node; p shows details; / filters this projection; d changes
direction; +/- change depth; l goes back; w copies a show command.
TAB and backtab move between nodes; s searches; g refreshes; q quits."
  (setq-local truncate-lines nil)
  (setq-local revert-buffer-function #'guixvis--refresh-graph)
  (guixvis--setup-native-buffer))

(defun guixvis--graph-object-p (value fields)
  "Return non-nil for an alist VALUE containing all FIELDS exactly once."
  (and (proper-list-p value)
       (cl-every (lambda (entry) (and (consp entry) (symbolp (car entry)))) value)
       (= (length value) (length (delete-dups (mapcar #'car value))))
       (cl-every (lambda (field) (assq field value)) fields)))

(defun guixvis--graph-kinds-p (kinds &optional nonempty)
  "Return non-nil for a bounded list of valid KINDS.
When NONEMPTY is non-nil require at least one dependency kind."
  (and (proper-list-p kinds) (<= (length kinds) 3)
       (or (not nonempty) kinds)
       (= (length kinds) (length (delete-dups (copy-sequence kinds))))
       (cl-every (lambda (kind) (member kind '("input" "propagated" "native"))) kinds)))

(defun guixvis--validate-graph (data)
  "Validate all of graph DATA against the current request before rendering.
Return the validated root node.  JSON false and null both decode to nil in
the shared legacy decoder; required field presence is checked separately."
  (unless (guixvis--graph-object-p
           data '(snapshot root_id root dir depth complete diagnostics_count
                  materialized truncated discovered_total discovery_complete
                  edges_total edges_truncated nodes edges))
    (error "Invalid graph response; press s to search again"))
  (let* ((ref (or guixvis--graph-root
                  `((name . ,guixvis--package-name) (id . ,guixvis--package-id)
                    (snapshot . ,guixvis--snapshot))))
         (nodes (alist-get 'nodes data)) (edges (alist-get 'edges data))
         (snapshot (alist-get 'snapshot data)) (root-id (alist-get 'root_id data))
         (by-id (make-hash-table :test 'eql))
         (seen-edges (make-hash-table :test 'equal)))
    (unless (and (proper-list-p nodes) (<= 1 (length nodes) 200)
                 (proper-list-p edges) (<= (length edges) 3000)
                 (natnump root-id) (<= root-id #xffffffff)
                 (equal (alist-get 'root data) (alist-get 'name ref))
                 (equal (alist-get 'dir data) guixvis--graph-direction)
                 (member guixvis--graph-direction '("deps" "reverse"))
                 (integerp guixvis--graph-depth) (<= 1 guixvis--graph-depth 8)
                 (equal (alist-get 'depth data) guixvis--graph-depth)
                 (or (null (alist-get 'id ref))
                     (and (equal root-id (alist-get 'id ref))
                          (equal snapshot (alist-get 'snapshot ref)))))
      (error "Unexpected or invalid graph response; press s to search again"))
    (guixvis--reference-path "graph" (alist-get 'root data) root-id snapshot)
    (dolist (field '(complete discovery_complete))
      (unless (memq (alist-get field data) '(nil t))
        (error "Invalid graph completeness metadata")))
    (dolist (field '(materialized truncated diagnostics_count edges_truncated))
      (unless (and (natnump (alist-get field data))
                   (<= (alist-get field data) #x100000000))
        (error "Invalid graph count")))
    (when (and (assq 'generation data) (not (natnump (alist-get 'generation data))))
      (error "Invalid graph generation"))
    (unless (and (= (alist-get 'materialized data) (length nodes))
                 (or (= 0 (alist-get 'truncated data)) (= (length nodes) 200))
                 (or (= 0 (alist-get 'edges_truncated data)) (= (length edges) 3000))
                 (eq (alist-get 'complete data)
                     (= 0 (alist-get 'diagnostics_count data))))
      (error "Inconsistent graph materialization or index completeness"))
    (let ((discovered (alist-get 'discovered_total data))
          (total (alist-get 'edges_total data))
          (omitted (alist-get 'edges_truncated data)))
      (unless (if (alist-get 'discovery_complete data)
                  (and (natnump discovered) (<= discovered #x100000000)
                       (= discovered (+ (length nodes) (alist-get 'truncated data))))
                (null discovered))
        (error "Inconsistent graph discovery count"))
      (unless (and (<= (+ (length nodes) (alist-get 'truncated data)) #x100000000)
                   (<= (+ (length edges) omitted) (* (length nodes) (length nodes)))
                   (or (null total)
                       (and (natnump total)
                            (= total (+ (length edges) omitted)))))
        (error "Inconsistent graph edge count")))
    (dolist (node nodes)
      (unless (guixvis--graph-object-p node '(id catalog snapshot name version degree kinds depth))
        (error "Invalid graph node"))
      (let ((id (alist-get 'id node)) (depth (alist-get 'depth node))
            (kind (alist-get 'kind node)))
        (unless (and (natnump id) (<= id #xffffffff) (not (gethash id by-id))
                     (equal snapshot (alist-get 'snapshot node))
                     (guixvis--valid-package-name-p (alist-get 'name node))
                     (stringp (alist-get 'version node))
                     (<= (length (alist-get 'version node)) 512)
                     (memq (alist-get 'catalog node) '(nil t))
                     (natnump (alist-get 'degree node))
                     (integerp depth) (<= 0 depth guixvis--graph-depth)
                     (eq (= depth 0) (= id root-id))
                     (guixvis--graph-kinds-p (alist-get 'kinds node))
                     (or (null kind) (member kind (alist-get 'kinds node))))
          (error "Invalid graph node identity or metadata"))
        (puthash id node by-id)))
    (let ((root (gethash root-id by-id)))
      (unless (and root (equal (alist-get 'name root) (alist-get 'root data)))
        (error "Graph root is missing or has another identity"))
      (dolist (edge edges)
        (unless (guixvis--graph-object-p edge '(from_id to_id from to kinds))
          (error "Invalid graph edge"))
        (let* ((from-id (alist-get 'from_id edge)) (to-id (alist-get 'to_id edge))
               (from (and (integerp from-id) (gethash from-id by-id)))
               (to (and (integerp to-id) (gethash to-id by-id)))
               (key (cons from-id to-id)))
          (unless (and from to (not (gethash key seen-edges))
                       (equal (alist-get 'from edge) (alist-get 'name from))
                       (equal (alist-get 'to edge) (alist-get 'name to))
                       (guixvis--graph-kinds-p (alist-get 'kinds edge) t))
            (error "Invalid or dangling graph edge identity"))
          (puthash key t seen-edges)))
      root)))

(defun guixvis--graph-label (node)
  "Return a readable, sanitized identity label for NODE."
  (format "%s@%s · ID %d · %s" (alist-get 'name node)
          (guixvis--clean-text (alist-get 'version node) t)
          (alist-get 'id node) (if (alist-get 'catalog node) "catalog" "private")))

(defun guixvis--graph-insert-node (node)
  "Insert a graph navigation button for validated NODE."
  (insert-text-button (guixvis--graph-label node)
                      'follow-link t 'guixvis-ref node
                      'action (lambda (button)
                                (goto-char (button-start button))
                                (guixvis-graph-follow))))

(defun guixvis--graph-restore-selection (id)
  "Select the first visible graph button for ID, falling back to the root."
  (goto-char (point-min))
  (let ((button (next-button (point-min))))
    (while (and button (not (equal id (alist-get 'id (button-get button 'guixvis-ref)))))
      (setq button (next-button (button-end button))))
    (if button (goto-char (button-start button))
      (when-let* ((first (next-button (point-min))))
        (goto-char (button-start first))))))

(defun guixvis--graph-print ()
  "Display the current validated graph and literal local filter."
  (let* ((data guixvis--graph-data)
         (nodes (alist-get 'nodes data)) (edges (alist-get 'edges data))
         (case-fold-search t)
         (visible (cl-remove-if-not
                   (lambda (node)
                     (string-match-p (regexp-quote guixvis--graph-filter)
                                     (concat (guixvis--graph-label node) " "
                                             (mapconcat #'identity (alist-get 'kinds node) ", "))))
                   nodes))
         (by-id (make-hash-table :test 'eql))
         (shown-edges nil)
         (selected guixvis--graph-selected-id)
         (inhibit-read-only t))
    (dolist (node visible) (puthash (alist-get 'id node) node by-id))
    (setq shown-edges (cl-remove-if-not
                       (lambda (edge) (and (gethash (alist-get 'from_id edge) by-id)
                                           (gethash (alist-get 'to_id edge) by-id))) edges))
    (erase-buffer)
    (insert (propertize "Guixvis dependency graph\n" 'face 'bold) "Root: ")
    (guixvis--graph-insert-node guixvis--package-data)
    (insert (format "\n%s · depth %d · snapshot %s\n"
                    (if (equal guixvis--graph-direction "deps") "Dependencies" "Dependents")
                    guixvis--graph-depth guixvis--snapshot))
    (insert (format "Projection: materialized %d · discovered %s · truncated %s\n"
                    (length nodes) (or (alist-get 'discovered_total data) "unknown")
                    (if (alist-get 'discovery_complete data)
                        (number-to-string (alist-get 'truncated data))
                      (format "at least %d (discovery incomplete)" (alist-get 'truncated data)))))
    (insert (format "Edges: materialized %d · total %s · truncated %s\n"
                    (length edges) (or (alist-get 'edges_total data) "unknown")
                    (if (alist-get 'edges_total data)
                        (number-to-string (alist-get 'edges_truncated data))
                      (format "at least %d (edge scan incomplete)" (alist-get 'edges_truncated data)))))
    (unless (alist-get 'complete data)
      (insert (format "Incomplete index: %d extraction diagnostics.\n"
                      (alist-get 'diagnostics_count data))))
    (insert (format "Local filter %S: %d/%d nodes · %d/%d edges visible\n"
                    guixvis--graph-filter (length visible) (length nodes)
                    (length shown-edges) (length edges)))
    (dotimes (depth (1+ guixvis--graph-depth))
      (let ((group (cl-remove-if-not (lambda (node) (= depth (alist-get 'depth node))) visible)))
        (when group
          (insert (propertize (format "\nDepth %d (%d)\n" depth (length group)) 'face 'bold))
          (dolist (node group)
            (insert "  ") (guixvis--graph-insert-node node)
            (insert (format " · degree %d · kinds %s\n" (alist-get 'degree node)
                            (if (alist-get 'kinds node)
                                (mapconcat #'identity (alist-get 'kinds node) ", ") "none")))))))
    (insert (propertize "\nAdjacency (actual package → dependency edges)\n" 'face 'bold))
    (if (null shown-edges) (insert "  No edges visible in this projection/filter.\n")
      (dolist (edge shown-edges)
        (insert "  ")
        (guixvis--graph-insert-node (gethash (alist-get 'from_id edge) by-id))
        (insert " → ")
        (guixvis--graph-insert-node (gethash (alist-get 'to_id edge) by-id))
        (insert " · " (mapconcat #'identity (alist-get 'kinds edge) ", ") "\n")))
    (insert "\nDepth groups are traversal distance; shared nodes and cycles are retained.\n")
    (guixvis--graph-restore-selection selected)
    (setq header-line-format
          "RET root · p details · / filter · d direction · +/- depth · l back · w copy show · s search · g refresh · q quit"
          mode-line-process nil)))

(defun guixvis--render-graph (data)
  "Atomically validate DATA, adopt its exact root, and display the graph."
  (let ((root (guixvis--validate-graph data)))
    (setq guixvis--graph-data data guixvis--graph-root root
          guixvis--package-data root guixvis--package-name (alist-get 'name root)
          guixvis--package-id (alist-get 'id root) guixvis--snapshot (alist-get 'snapshot root))
    (guixvis--graph-print)))

(defun guixvis--graph-clear-actions ()
  "Remove the projection and all references usable by package actions."
  (setq guixvis--graph-data nil guixvis--package-data nil
        guixvis--package-name nil guixvis--package-id nil guixvis--snapshot nil)
  (let ((inhibit-read-only t)) (erase-buffer)))

(defun guixvis--graph-error (text)
  "Invalidate graph actions and history, then show failure TEXT."
  (guixvis--graph-clear-actions)
  (setq guixvis--graph-history nil guixvis--graph-selected-id nil)
  (when (get-text-property 0 'guixvis-stale text)
    (setq guixvis--graph-root nil))
  (let ((inhibit-read-only t)) (insert (guixvis--clean-text text t) "\n"))
  (guixvis--show-error text))

(defun guixvis--refresh-graph (&rest _ignored)
  "Refresh the exact graph context, dropping any old projection actions."
  (let* ((ref (or guixvis--graph-root
                  `((name . ,guixvis--package-name) (id . ,guixvis--package-id)
                    (snapshot . ,guixvis--snapshot))))
         (path (guixvis--reference-path "graph" (alist-get 'name ref)
                                        (alist-get 'id ref) (alist-get 'snapshot ref))))
    (unless (and (member guixvis--graph-direction '("deps" "reverse"))
                 (integerp guixvis--graph-depth) (<= 1 guixvis--graph-depth 8))
      (user-error "Invalid graph direction or depth"))
    (when guixvis--graph-data
      (setq guixvis--graph-selected-id (alist-get 'id (guixvis--ref-at-point))))
    (setq guixvis--graph-root ref)
    (guixvis--graph-clear-actions)
    (let ((inhibit-read-only t)) (insert "Loading graph…\n"))
    (guixvis--fetch (format "%s%sdir=%s&depth=%d&budget=200"
                            path (if (string-match-p "?" path) "&" "?")
                            guixvis--graph-direction guixvis--graph-depth)
                    #'guixvis--render-graph #'guixvis--graph-error)))

;;;###autoload
(defun guixvis-graph (name &optional id snapshot)
  "Open a native graph for NAME with optional exact ID and SNAPSHOT."
  (interactive (list (read-string "Guix graph root: " (guixvis--name-at-point))))
  (guixvis--reference-path "graph" name id snapshot)
  (guixvis--base-url)
  (pop-to-buffer (get-buffer-create "*Guixvis graph*"))
  (unless (derived-mode-p 'guixvis-graph-mode) (guixvis-graph-mode))
  (setq guixvis--graph-root `((name . ,name) (id . ,id) (snapshot . ,snapshot))
        guixvis--graph-direction "deps" guixvis--graph-depth 1
        guixvis--graph-filter "" guixvis--graph-history nil
        guixvis--graph-selected-id id guixvis--graph-data nil)
  (guixvis--refresh-graph))

;;;###autoload
(defun guixvis-graph-at-point ()
  "Open a native graph preserving the exact reference selected at point."
  (interactive)
  (let ((ref (guixvis--ref-at-point)))
    (unless ref (user-error "Move to a package first"))
    (guixvis-graph (alist-get 'name ref) (alist-get 'id ref) (alist-get 'snapshot ref))))

(defun guixvis--graph-current-ref ()
  "Return the current validated graph reference, or report no projection."
  (unless guixvis--graph-data (user-error "No graph projection; press s to search again"))
  (guixvis--ref-at-point))

(defun guixvis-graph-follow ()
  "Follow the selected node as a graph root, saving a bounded context."
  (interactive)
  (let ((ref (guixvis--graph-current-ref)))
    (push (list guixvis--graph-root guixvis--graph-direction guixvis--graph-depth
                guixvis--graph-filter (alist-get 'id ref)) guixvis--graph-history)
    (when (> (length guixvis--graph-history) 32)
      (setcdr (nthcdr 31 guixvis--graph-history) nil))
    (setq guixvis--graph-root ref guixvis--graph-filter ""
          guixvis--graph-selected-id (alist-get 'id ref))
    (guixvis--refresh-graph)))

(defun guixvis-graph-back ()
  "Restore the previous exact graph context and selected variant."
  (interactive)
  (if (null guixvis--graph-history) (progn (message "No previous graph") nil)
    (pcase-let ((`(,root ,direction ,depth ,filter ,selected) (pop guixvis--graph-history)))
      (setq guixvis--graph-root root guixvis--graph-direction direction
            guixvis--graph-depth depth guixvis--graph-filter filter
            guixvis--graph-selected-id selected guixvis--graph-data nil)
      (guixvis--refresh-graph))))

(defun guixvis-graph-filter (text)
  "Filter only the current projection with case-insensitive literal TEXT."
  (interactive (list (read-string "Local graph filter: " guixvis--graph-filter)))
  (let ((ref (guixvis--graph-current-ref)))
    (unless (and (stringp text) (<= (length text) 200))
      (user-error "Graph filter must be a string of at most 200 characters"))
    (setq guixvis--graph-filter (guixvis--clean-text text t)
          guixvis--graph-selected-id (alist-get 'id ref))
    (guixvis--graph-print)))

(defun guixvis-graph-toggle-direction ()
  "Toggle dependency and reverse dependency traversal for this root."
  (interactive)
  (setq guixvis--graph-direction (if (equal guixvis--graph-direction "deps") "reverse" "deps"))
  (guixvis--refresh-graph))

(defun guixvis--graph-change-depth (delta)
  "Change graph depth by DELTA within the supported range 1 through 8."
  (let ((depth (+ guixvis--graph-depth delta)))
    (if (<= 1 depth 8)
        (progn (setq guixvis--graph-depth depth) (guixvis--refresh-graph))
      (message "Graph depth must stay between 1 and 8"))))

(defun guixvis-graph-increase-depth ()
  "Increase graph traversal depth, up to eight."
  (interactive) (guixvis--graph-change-depth 1))

(defun guixvis-graph-decrease-depth ()
  "Decrease graph traversal depth, down to one."
  (interactive) (guixvis--graph-change-depth -1))

(defun guixvis-graph-copy-command ()
  "Copy a validated show command for the selected graph node."
  (interactive)
  (guixvis--graph-current-ref)
  (guixvis-copy-command "show"))

(provide 'guixvis-graph)
;;; guixvis-graph.el ends here
