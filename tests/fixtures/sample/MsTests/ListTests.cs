namespace MsTests;

[TestClass]
public sealed class ListTests
{
    [TestMethod]
    public void Counts() => Assert.AreEqual(3, new List<int> { 1, 2, 3 }.Count);

    [TestMethod]
    [Ignore("demonstrates a skipped test")]
    public void Skipped() => Assert.Fail();

    [TestMethod]
    public void FailsOnDemand() =>
        Assert.AreNotEqual("1", Environment.GetEnvironmentVariable("ROUILLE_FIXTURE_FAIL"), "fixture failure requested");
}
