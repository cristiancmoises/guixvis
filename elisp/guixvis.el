;;; guixvis.el --- Browse Guix packages from Emacs  -*- lexical-binding: t; -*-

;; Copyright © 2026 Cristian Cezar Moisés <cristiancmoises@users.noreply.github.com>

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Version: 0.6.0
;; Package-Requires: ((emacs "27.1"))

;;; Commentary:

;; guixvis is an interactive package explorer and dependency visualizer for
;; GNU Guix — a terminal UI plus a local web interface; see
;; <https://codeberg.org/berkeley/guixvis>.
;;
;; `guixvis' opens the terminal interface and `guixvis-web' opens a browser.
;; Start "guixvis web", then use `guixvis-search' for asynchronous native
;; browsing.  RET opens package details; w copies a Guix command for review.
;; This library never installs or removes packages.  The native buffers use
;; your current Emacs theme and need no packages beyond bundled libraries.
;; `guixvis-popup-install' adds the terminal and browser commands to the
;; `guix' popup when Emacs-Guix is installed:
;;
;;   (require 'guixvis)
;;   (guixvis-popup-install)

;;; Code:

(require 'browse-url)
(require 'button)
(require 'cl-lib)
(require 'json)
(require 'subr-x)
(require 'tabulated-list)
(require 'term)
(require 'url)
(require 'url-http)
(require 'url-parse)

(defvar url-http-response-status)
(defvar url-http-end-of-headers)

(defgroup guixvis nil
  "Run the guixvis package explorer from Emacs."
  :group 'tools
  :prefix "guixvis-")

(defcustom guixvis-program
  (or (executable-find "guixvis") "guixvis")
  "Name of the \"guixvis\" executable.
Set this to the full file name when guixvis is not on `exec-path'."
  :type 'string
  :group 'guixvis)

(defcustom guixvis-arguments nil
  "Additional arguments passed to `guixvis-program'."
  :type '(repeat string)
  :group 'guixvis)

(defcustom guixvis-web-url "http://127.0.0.1:8787"
  "Loopback URL of the service started with \"guixvis web\".
Use HTTP or HTTPS with 127.0.0.1, localhost, or [::1], and an optional port.
Credentials, paths, queries, and fragments are not accepted."
  :type 'string
  :group 'guixvis)

(defcustom guixvis-search-limit 100
  "Maximum number of native search results, between 1 and 500."
  :type '(integer :tag "Result limit")
  :group 'guixvis)

(defcustom guixvis-request-timeout 10
  "Seconds to wait for a response from the local web service."
  :type 'number
  :group 'guixvis)

(defvar guixvis-search-history nil
  "Minibuffer history for native package searches.")

(defvar-local guixvis--query "")
(defvar-local guixvis--package-name nil)
(defvar-local guixvis--request-generation 0)
(defvar-local guixvis--pending-request nil)

(cl-defstruct (guixvis--request-state
               (:constructor guixvis--make-request-state))
  buffer timer done success failure)

(defun guixvis--base-url ()
  "Return the validated service URL without a trailing slash."
  (unless (and (stringp guixvis-web-url)
               (let ((case-fold-search t))
                 (string-match-p
                  (concat "\\`https?://\\(?:127\\.0\\.0\\.1\\|localhost"
                          "\\|\\[::1\\]\\)\\(?::[0-9]+\\)?/?\\'")
                  guixvis-web-url)))
    (user-error "Set `guixvis-web-url' to a loopback HTTP(S) URL without credentials or a path"))
  (let* ((parsed (url-generic-parse-url guixvis-web-url))
         (port (url-port parsed)))
    (unless (and (integerp port) (<= 1 port 65535))
      (user-error "Invalid port in `guixvis-web-url'")))
  (string-remove-suffix "/" guixvis-web-url))

(defun guixvis--valid-package-name-p (name)
  "Return non-nil when NAME is a safe Guix package name."
  (and (stringp name)
       (<= (length name) 128)
       (let ((case-fold-search nil))
         (string-match-p "\\`[A-Za-z0-9][A-Za-z0-9+._-]*\\'" name))))

(defun guixvis--clean-text (text &optional single-line)
  "Remove terminal control characters from TEXT.
When SINGLE-LINE is non-nil, replace newlines and tabs with spaces."
  (unless (stringp text)
    (error "Invalid text in package response"))
  (replace-regexp-in-string
   (if single-line "[[:cntrl:]]" "[\0-\10\13-\37\177]") " " text))

(defun guixvis--connection-help (reason)
  "Describe REASON and how to connect to Guixvis."
  (format "%s. Start `guixvis web', check `guixvis-web-url', then press g"
          reason))

(defun guixvis--dispose-request (request)
  "Cancel REQUEST and release its timer and response buffer."
  (when request
    (setf (guixvis--request-state-done request) t)
    (when (timerp (guixvis--request-state-timer request))
      (cancel-timer (guixvis--request-state-timer request)))
    (let ((buffer (guixvis--request-state-buffer request)))
      (when (buffer-live-p buffer)
        (let ((process (get-buffer-process buffer)))
          (when (process-live-p process)
            (delete-process process)))
        (kill-buffer buffer)))))

(defun guixvis--cancel-pending-request ()
  "Release the request owned by the current Guixvis buffer."
  (guixvis--dispose-request guixvis--pending-request)
  (setq guixvis--pending-request nil))

(defun guixvis--response-data (status)
  "Read the JSON response in the current URL buffer, checking STATUS."
  (cond
   ((and (integerp url-http-response-status)
         (= url-http-response-status 503))
    (error "The package index is still building; wait a moment and press g"))
   ((and (integerp url-http-response-status)
         (= url-http-response-status 404))
    (error "Package not found; press s to search again"))
   ((and (integerp url-http-response-status)
         (<= 300 url-http-response-status 399))
    (error "The service redirected the request; configure its direct loopback URL"))
   ((plist-get status :error)
    (error "%s" (guixvis--connection-help "Cannot reach the local service")))
   ((not (eq url-http-response-status 200))
    (error "Local service returned HTTP %s" url-http-response-status)))
  (unless (and (bound-and-true-p url-http-end-of-headers)
               (<= (buffer-size) (* 8 1024 1024)))
    (error "Invalid or oversized response from the local service"))
  (goto-char url-http-end-of-headers)
  (condition-case nil
      (json-parse-buffer :object-type 'alist :array-type 'list
                         :null-object nil :false-object nil)
    (error (error "The local service returned invalid JSON"))))

(defun guixvis--request (path success failure)
  "Fetch API PATH asynchronously and call SUCCESS or FAILURE.
SUCCESS receives parsed JSON; FAILURE receives a readable error string.
Requests bypass proxies, omit cookies, and do not follow redirects."
  (let ((address (concat (guixvis--base-url) "/api/v1/" path))
        (request (guixvis--make-request-state :success success :failure failure))
        (url-proxy-services '(("no_proxy" . ".*")))
        (url-request-extra-headers '(("Accept" . "application/json")))
        (url-request-method "GET")
        (url-request-data nil)
        (url-max-redirections 0))
    (unless (and (numberp guixvis-request-timeout)
                 (> guixvis-request-timeout 0))
      (user-error "Set `guixvis-request-timeout' to a positive number"))
    (condition-case err
        (let ((buffer
               (url-retrieve
                address
                (lambda (status)
                  (unless (guixvis--request-state-done request)
                    (setf (guixvis--request-state-done request) t)
                    (when (timerp (guixvis--request-state-timer request))
                      (cancel-timer (guixvis--request-state-timer request)))
                    (unwind-protect
                        (condition-case response-error
                            (funcall success (guixvis--response-data status))
                          (error (funcall failure
                                          (error-message-string response-error))))
                      (kill-buffer (current-buffer)))))
                nil t t)))
          (unless (or buffer (guixvis--request-state-done request))
            (error "Cannot create a connection to the local service"))
          (setf (guixvis--request-state-buffer request) buffer)
          (unless (guixvis--request-state-done request)
            (with-current-buffer buffer
              ;; URL parsing happens later, outside the dynamic bindings above.
              (setq-local url-max-redirections 0))
            (setf (guixvis--request-state-timer request)
                  (run-at-time
                   guixvis-request-timeout nil
                   (lambda ()
                     (unless (guixvis--request-state-done request)
                       (guixvis--dispose-request request)
                       (funcall failure
                                (guixvis--connection-help "The request timed out"))))))))
      (error
       (guixvis--dispose-request request)
       (funcall failure (guixvis--connection-help (error-message-string err)))))
    request))

(defun guixvis--show-error (text)
  "Show TEXT in the current native buffer and the echo area."
  (setq header-line-format (propertize text 'face 'error)
        mode-line-process " [error]")
  (message "Guixvis: %s" text))

(defun guixvis--fetch (path render)
  "Fetch PATH for the current buffer and call RENDER for its latest request."
  (guixvis--cancel-pending-request)
  (cl-incf guixvis--request-generation)
  (let ((target (current-buffer))
        (generation guixvis--request-generation))
    (setq header-line-format "Loading packages…"
          mode-line-process " [loading]")
    (setq guixvis--pending-request
          (guixvis--request
           path
           (lambda (data)
             (when (buffer-live-p target)
               (with-current-buffer target
                 (when (= generation guixvis--request-generation)
                   (funcall render data)))))
           (lambda (error-text)
             (when (buffer-live-p target)
               (with-current-buffer target
                 (when (= generation guixvis--request-generation)
                   (guixvis--show-error error-text)))))))))

(defun guixvis--terminal (program args)
  "Run PROGRAM with ARGS, reusing the live \"*guixvis*\" terminal."
  (let ((buffer (get-buffer-create "*guixvis*")))
    (unless (comint-check-proc buffer)
      (with-current-buffer buffer
        (term-mode))
      (term-exec buffer "guixvis" program nil args))
    (switch-to-buffer buffer)
    (with-current-buffer buffer
      (term-char-mode))
    buffer))

;;;###autoload
(defun guixvis (&optional args)
  "Run guixvis, the interactive Guix package explorer, in a terminal buffer.
With a prefix argument, prompt for additional command line arguments
\(see `guixvis-arguments').  A running terminal is reused; quit its process
before launching with different arguments."
  (interactive
   (list (and current-prefix-arg
              (split-string-and-unquote (read-string "Arguments for guixvis: ")))))
  (unless (or (file-executable-p guixvis-program)
              (executable-find guixvis-program))
    (user-error "guixvis not found; install it or set `guixvis-program'"))
  (guixvis--terminal guixvis-program (append guixvis-arguments args)))

;;;###autoload
(defun guixvis-web ()
  "Open the guixvis web interface in a browser.
Start it first with \"guixvis web\"; see `guixvis-web-url'."
  (interactive)
  (browse-url (guixvis--base-url)))

(defvar guixvis-search-mode-map
  (let ((map (make-sparse-keymap)))
    (set-keymap-parent map tabulated-list-mode-map)
    (define-key map (kbd "RET") #'guixvis-package-at-point)
    (define-key map (kbd "s") #'guixvis-search)
    (define-key map (kbd "/") #'guixvis-search)
    (define-key map (kbd "w") #'guixvis-copy-command)
    (define-key map (kbd "b") #'guixvis-web)
    map)
  "Keymap for native Guixvis search results.")

(define-derived-mode guixvis-search-mode tabulated-list-mode "Guixvis"
  "Browse Guix packages asynchronously through the local service.
RET shows details, s searches, g refreshes, and w copies a Guix command."
  (setq tabulated-list-format
        [("Package" 26 t) ("Version" 16 t)
         ("Deps" 6 guixvis--sort-dependencies :right-align t)
         ("Used by" 8 guixvis--sort-dependents :right-align t)
         ("Synopsis" 0 t)])
  (setq tabulated-list-padding 2
        tabulated-list-sort-key nil)
  (setq-local revert-buffer-function #'guixvis--refresh-search)
  (add-hook 'kill-buffer-hook #'guixvis--cancel-pending-request nil t)
  (tabulated-list-init-header))

(defun guixvis--sort-dependencies (a b)
  "Compare the dependency counts of table entries A and B."
  (< (string-to-number (aref (cadr a) 2))
     (string-to-number (aref (cadr b) 2))))

(defun guixvis--sort-dependents (a b)
  "Compare the dependent counts of table entries A and B."
  (< (string-to-number (aref (cadr a) 3))
     (string-to-number (aref (cadr b) 3))))

(defun guixvis--search-entry (item)
  "Validate a search ITEM and turn it into a table entry."
  (let ((name (alist-get 'name item))
        (deps (alist-get 'deps item))
        (dependents (alist-get 'dependents item)))
    (unless (and (guixvis--valid-package-name-p name)
                 (natnump deps) (natnump dependents))
      (error "Invalid package entry from the local service"))
    (list name (vector name
                       (guixvis--clean-text (alist-get 'version item) t)
                       (number-to-string deps)
                       (number-to-string dependents)
                       (guixvis--clean-text (alist-get 'synopsis item) t)))))

(defun guixvis--render-search (data)
  "Display the search response DATA in the current buffer."
  (unless (and (listp data) (assq 'items data)
               (listp (alist-get 'items data)))
    (error "Invalid search response from the local service"))
  (let ((entries (mapcar #'guixvis--search-entry (alist-get 'items data))))
    (setq tabulated-list-entries entries)
    (tabulated-list-print t)
    (tabulated-list-init-header)
    (setq mode-line-process
          (format " [%d%s packages]" (length entries)
                  (if (alist-get 'capped data) "+" "")))
    (message "Guixvis: %d%s packages for %S; RET details, s search, g refresh, w copy command"
             (length entries) (if (alist-get 'capped data) "+" "") guixvis--query)))

(defun guixvis--refresh-search (&rest _ignored)
  "Refresh results for the current native search."
  (unless (and (integerp guixvis-search-limit)
               (<= 1 guixvis-search-limit 500))
    (user-error "Set `guixvis-search-limit' between 1 and 500"))
  (guixvis--fetch
   (format "search?q=%s&limit=%d" (url-hexify-string guixvis--query)
           guixvis-search-limit)
   #'guixvis--render-search))

;;;###autoload
(defun guixvis-search (query)
  "Search for QUERY using the local Guixvis service without blocking Emacs.
Start \"guixvis web\" first.  An empty query lists packages up to
`guixvis-search-limit'.  Searches are limited to 200 characters."
  (interactive (list (read-string "Guix packages: " guixvis--query
                                 'guixvis-search-history)))
  (unless (and (stringp query) (<= (length query) 200))
    (user-error "Search queries must be strings of at most 200 characters"))
  (guixvis--base-url)
  (let ((buffer (get-buffer-create "*Guixvis packages*")))
    (pop-to-buffer buffer)
    (unless (derived-mode-p 'guixvis-search-mode)
      (guixvis-search-mode))
    (setq guixvis--query query
          tabulated-list-entries nil)
    (tabulated-list-print)
    (guixvis--refresh-search)))

(defvar guixvis-package-mode-map
  (let ((map (make-sparse-keymap)))
    (set-keymap-parent map special-mode-map)
    (define-key map (kbd "s") #'guixvis-search)
    (define-key map (kbd "g") #'guixvis--refresh-package)
    (define-key map (kbd "w") #'guixvis-copy-command)
    (define-key map (kbd "b") #'guixvis-web)
    (define-key map (kbd "TAB") #'forward-button)
    (define-key map (kbd "<backtab>") #'backward-button)
    map)
  "Keymap for native Guixvis package details.")

(define-derived-mode guixvis-package-mode special-mode "Guixvis Package"
  "Show a Guix package and follow its dependency links.
TAB moves between dependencies, RET follows a link, and w copies a command."
  (setq-local truncate-lines nil)
  (setq-local revert-buffer-function #'guixvis--refresh-package)
  (add-hook 'kill-buffer-hook #'guixvis--cancel-pending-request nil t))

(defun guixvis--insert-package-links (heading packages)
  "Insert HEADING and buttons for PACKAGES."
  (unless (listp packages)
    (error "Invalid dependency list from the local service"))
  (insert (propertize (format "\n%s (%d)\n" heading (length packages))
                      'face 'bold))
  (if (null packages)
      (insert "  None\n")
    (dolist (package packages)
      (let ((name (alist-get 'name package))
            (kind (alist-get 'kind package)))
        (unless (guixvis--valid-package-name-p name)
          (error "Invalid dependency name from the local service"))
        (insert "  ")
        (insert-text-button
         name 'follow-link t 'guixvis-package name
         'action (lambda (button)
                   (guixvis-package (button-get button 'guixvis-package))))
        (when kind
          (insert "  (" (guixvis--clean-text kind t) ")"))
        (insert "\n")))))

(defun guixvis--render-package (data)
  "Display package response DATA in the current buffer."
  (unless (and (listp data)
               (equal (alist-get 'name data) guixvis--package-name))
    (error "Unexpected package response from the local service"))
  (let ((inhibit-read-only t))
    (erase-buffer)
    (insert (propertize guixvis--package-name 'face 'bold) "  "
            (guixvis--clean-text (alist-get 'version data) t) "\n\n"
            (guixvis--clean-text (alist-get 'synopsis data)) "\n\n"
            (guixvis--clean-text (or (alist-get 'description data) "")) "\n\n")
    (insert "License: "
            (mapconcat (lambda (license) (guixvis--clean-text license t))
                       (alist-get 'licenses data) ", ") "\n"
            "Homepage: " (guixvis--clean-text (or (alist-get 'homepage data) "") t)
            "\nSource: " (guixvis--clean-text (or (alist-get 'file data) "") t))
    (when (natnump (alist-get 'line data))
      (insert (format ":%d" (alist-get 'line data))))
    (insert "\n")
    (guixvis--insert-package-links "Dependencies" (alist-get 'deps data))
    (guixvis--insert-package-links "Dependents" (alist-get 'dependents data))
    (guixvis--insert-package-links "Same module" (alist-get 'module_neighbors data))
    (goto-char (point-min)))
  (setq header-line-format
        "TAB / RET follow package links  ·  w copy command  s search  g refresh  q quit"
        mode-line-process nil))

(defun guixvis--refresh-package (&rest _ignored)
  "Refresh the package shown in the current details buffer."
  (unless (guixvis--valid-package-name-p guixvis--package-name)
    (user-error "No package selected"))
  (guixvis--fetch (concat "package/" (url-hexify-string guixvis--package-name))
                 #'guixvis--render-package))

;;;###autoload
(defun guixvis-package (name)
  "Show details for the Guix package NAME asynchronously."
  (interactive (list (read-string "Guix package: " (guixvis--name-at-point))))
  (unless (guixvis--valid-package-name-p name)
    (user-error "Invalid Guix package name"))
  (guixvis--base-url)
  (let ((buffer (get-buffer-create "*Guixvis package*")))
    (pop-to-buffer buffer)
    (unless (derived-mode-p 'guixvis-package-mode)
      (guixvis-package-mode))
    (setq guixvis--package-name name)
    (let ((inhibit-read-only t))
      (erase-buffer)
      (insert "Loading " name "…\n"))
    (guixvis--refresh-package)))

(defun guixvis--name-at-point ()
  "Return the package selected at point, or nil."
  (or (and (derived-mode-p 'guixvis-search-mode) (tabulated-list-get-id))
      (let ((button (button-at (point))))
        (and button (button-get button 'guixvis-package)))
      guixvis--package-name))

(defun guixvis-package-at-point ()
  "Open details for the package at point."
  (interactive)
  (let ((name (guixvis--name-at-point)))
    (unless name (user-error "Move to a package row first"))
    (guixvis-package name)))

(defun guixvis--command (action name)
  "Return a safely quoted Guix ACTION for package NAME."
  (unless (member action '("install" "remove" "show" "shell"))
    (user-error "Choose install, remove, show, or shell"))
  (unless (guixvis--valid-package-name-p name)
    (user-error "Invalid Guix package name"))
  ;; Names cannot contain quotes or begin with an option.  Single quotes
  ;; prevent shell expansion; `guix shell' reserves -- for its command.
  (format "guix %s %s'%s'" action (if (equal action "shell") "" "-- ") name))

;;;###autoload
(defun guixvis-copy-command (action)
  "Copy a Guix ACTION for the selected package to the kill ring.
Choose install, remove, show, or shell.  This never executes the command."
  (interactive (list (completing-read "Copy Guix command: "
                                     '("install" "remove" "show" "shell")
                                     nil t nil nil "install")))
  (let ((name (guixvis--name-at-point)))
    (unless name (user-error "No package selected"))
    (let ((command (guixvis--command action name)))
      (kill-new command)
      (message "Copied; review before running: %s" command))))

(defvar guixvis-popup-installed nil
  "Non-nil once the guixvis entries were added to the `guix' popup.")

;;;###autoload
(defun guixvis-popup-install ()
  "Add the guixvis commands to the Emacs-Guix `guix' popup.
They land in the \"Miscellaneous commands\" group, bound to \"v\" (the
terminal UI) and \"V\" (the web interface).  Requires Emacs-Guix; does
nothing useful without it."
  (interactive)
  (require 'transient nil t)
  (require 'guix-popup nil t)
  (cond
   ((not (fboundp 'transient-insert-suffix))
    (user-error "`guixvis-popup-install' needs Emacs-Guix and `transient'"))
   ((not (fboundp 'guix-popup))
    (user-error "The `guix' popup is not available; is Emacs-Guix installed?"))
   (guixvis-popup-installed
    (message "guixvis is already in the guix popup"))
   (t
    ;; One call per entry: handing `transient-append-suffix' the whole list
    ;; makes it parse the lot as a single suffix.
    (if (fboundp 'transient-append-suffix)
        (progn
          (transient-append-suffix 'guix-popup "E" '("v" "guixvis" guixvis))
          (transient-append-suffix 'guix-popup "v" '("V" "guixvis web" guixvis-web)))
      ;; Older `transient' releases have no append variant.
      (transient-insert-suffix 'guix-popup "H" '("V" "guixvis web" guixvis-web))
      (transient-insert-suffix 'guix-popup "H" '("v" "guixvis" guixvis)))
    (setq guixvis-popup-installed t)
    (message "guixvis added to the guix popup (v / V)"))))

(provide 'guixvis)

;;; guixvis.el ends here
