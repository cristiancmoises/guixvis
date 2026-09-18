;;; guixvis.el --- Launch the guixvis package explorer from Emacs  -*- lexical-binding: t; -*-

;; Copyright © 2026 Cristian Cezar Moisés <cristiancmoises@users.noreply.github.com>

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; guixvis is an interactive package explorer and dependency visualizer for
;; GNU Guix — a terminal UI plus a local web interface; see
;; <https://codeberg.org/berkeley/guixvis>.
;;
;; This file ships with guixvis, so the commands are only offered to users who
;; actually have the program.  It provides `guixvis' (TUI) and `guixvis-web'
;; (the local web interface), and `guixvis-popup-install', which adds both to
;; the `guix' popup when Emacs-Guix is installed:
;;
;;   (require 'guixvis)
;;   (guixvis-popup-install)

;;; Code:

(require 'browse-url)
(require 'term)

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
  "URL of the guixvis web interface, served by \"guixvis web\"."
  :type 'string
  :group 'guixvis)

(defun guixvis--terminal (program args)
  "Run PROGRAM with ARGS in a `term' buffer named \"*PROGRAM*\"."
  (let ((buffer (get-buffer-create (concat "*" program "*"))))
    (unless (comint-check-proc buffer)
      (with-current-buffer buffer
        (term-mode)))
    (term-exec buffer program program nil args)
    (switch-to-buffer buffer)
    (with-current-buffer buffer
      (term-char-mode))
    buffer))

;;;###autoload
(defun guixvis (&optional args)
  "Run guixvis, the interactive Guix package explorer, in a terminal buffer.
With a prefix argument, prompt for additional command line arguments
\(see `guixvis-arguments')."
  (interactive
   (list (and current-prefix-arg
              (split-string (read-string "Arguments for guixvis: ") nil t))))
  (unless (or (file-executable-p guixvis-program)
              (executable-find guixvis-program))
    (user-error "guixvis not found; install it or set `guixvis-program'"))
  (guixvis--terminal guixvis-program (append guixvis-arguments args)))

;;;###autoload
(defun guixvis-web ()
  "Open the guixvis web interface in a browser.
Start it first with \"guixvis web\"; see `guixvis-web-url'."
  (interactive)
  (browse-url guixvis-web-url))

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
