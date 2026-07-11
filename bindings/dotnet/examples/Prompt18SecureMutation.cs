using Oxide.Sdk;

if (args.Length != 1)
{
    Console.Error.WriteLine("usage: Prompt18SecureMutation input.pdf");
    return 2;
}

using var document = OxideDocument.Open(File.ReadAllBytes(args[0]));
Console.WriteLine(document.Prompt18ReportJson());
Console.WriteLine(document.AssociatedFilesReportJson());
Console.WriteLine(document.EditPolicyReportJson("attachment_remove"));

var addOptions = """{"filename":"evidence.txt","mime":"text/plain","relationship":"data","deterministic":true}""";
var added = document.AssociatedFileAdd("bounded evidence"u8.ToArray(), addOptions);
File.WriteAllBytes("prompt18-associated.pdf", added.Bytes);
return 0;
