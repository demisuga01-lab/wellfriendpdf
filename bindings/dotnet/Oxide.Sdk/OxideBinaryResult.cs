namespace Oxide.Sdk;

public sealed record OxideBinaryResult(byte[] Bytes, string ReportJson)
{
    public void WriteBytes(string path)
    {
        ArgumentNullException.ThrowIfNull(path);
        File.WriteAllBytes(path, Bytes);
    }
}
