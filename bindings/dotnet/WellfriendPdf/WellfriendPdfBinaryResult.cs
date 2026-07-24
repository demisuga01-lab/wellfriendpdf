namespace WellfriendPdf;

public sealed record WellfriendBinaryResult(byte[] Bytes, string ReportJson)
{
    public void WriteBytes(string path)
    {
        ArgumentNullException.ThrowIfNull(path);
        File.WriteAllBytes(path, Bytes);
    }
}
