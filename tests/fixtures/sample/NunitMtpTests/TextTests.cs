namespace NunitMtpTests;

public class TextTests
{
    [Test]
    public void Uppercases() => Assert.That("abc".ToUpperInvariant(), Is.EqualTo("ABC"));

    [TestCase("a", 1)]
    [TestCase("abc", 3)]
    public void Lengths(string s, int n) => Assert.That(s.Length, Is.EqualTo(n));

    [Test]
    public void FailsOnDemand() =>
        Assert.That(Environment.GetEnvironmentVariable("ROUILLE_FIXTURE_FAIL"), Is.Not.EqualTo("1"), "fixture failure requested");
}
