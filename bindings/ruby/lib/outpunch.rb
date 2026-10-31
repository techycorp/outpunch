# frozen_string_literal: true

# A precompiled gem ships one binary per Ruby minor version; a source gem compiles a single one.
precompiled = File.join(__dir__, "outpunch", RUBY_VERSION[/\A\d+\.\d+/])
require File.join(File.directory?(precompiled) ? precompiled : File.join(__dir__, "outpunch"), "outpunch")
