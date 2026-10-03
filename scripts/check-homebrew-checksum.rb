# Run with `brew ruby scripts/check-homebrew-checksum.rb` on a Mac with Homebrew.
# Exercises Homebrew's real download-integrity verifier without touching a tap
# or fetching/replacing any release artifact.
require "digest"
require "downloadable"
require "tmpdir"

Dir.mktmpdir("bullseye-homebrew-checksum-") do |dir|
  fixture = Pathname.new(dir) / "asset.tar.gz"
  fixture.write("original asset bytes")
  expected = Checksum.new(Digest::SHA256.file(fixture).hexdigest)
  Downloadable.verification_cache.verify(fixture, expected)

  fixture.write("tampered asset bytes")
  begin
    Downloadable.verification_cache.verify(fixture, expected)
  rescue ChecksumMismatchError
    puts "PASS: Homebrew accepted matching digest and rejected changed bytes (ChecksumMismatchError)"
  else
    abort "FAIL: Homebrew accepted a mismatched asset"
  end
end
