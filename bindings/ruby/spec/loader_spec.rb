# frozen_string_literal: true

require "fileutils"
require "open3"
require "rbconfig"
require "tmpdir"

RSpec.describe "lib/outpunch.rb" do
  loader = File.expand_path("../lib/outpunch.rb", __dir__)
  ruby_minor = RUBY_VERSION[/\A\d+\.\d+/]

  def write(path, content)
    FileUtils.mkdir_p(File.dirname(path))
    File.write(path, content)
  end

  define_method(:load_in) do |dir|
    FileUtils.cp(loader, dir)
    Open3.capture3(RbConfig.ruby, "-e", "require #{File.join(dir, 'outpunch').inspect}; print LOADED")
  end

  it "loads the binary built for the running Ruby from a precompiled gem" do
    Dir.mktmpdir do |dir|
      write("#{dir}/outpunch/#{ruby_minor}/outpunch.rb", "LOADED = 'precompiled'")
      write("#{dir}/outpunch/outpunch.rb", "LOADED = 'source'")

      stdout, = load_in(dir)

      expect(stdout).to eq("precompiled")
    end
  end

  it "loads the binary compiled on install from a source gem" do
    Dir.mktmpdir do |dir|
      write("#{dir}/outpunch/outpunch.rb", "LOADED = 'source'")

      stdout, = load_in(dir)

      expect(stdout).to eq("source")
    end
  end

  it "surfaces a load failure in the precompiled binary" do
    Dir.mktmpdir do |dir|
      write("#{dir}/outpunch/#{ruby_minor}/outpunch.rb", "raise LoadError, 'libruby mismatch'")
      write("#{dir}/outpunch/outpunch.rb", "LOADED = 'source'")

      _, stderr, status = load_in(dir)

      expect(status).not_to be_success
      expect(stderr).to include("libruby mismatch")
    end
  end
end
