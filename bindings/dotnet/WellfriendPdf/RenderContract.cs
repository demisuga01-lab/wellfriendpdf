using System.Text.Json;
using System.Text.Json.Serialization;

namespace WellfriendPdf;

public sealed record RenderContract
{
    private const uint CurrentSchemaVersion = 1;

    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        Converters = { new JsonStringEnumConverter() },
    };

    [JsonPropertyName("schema_version")]
    public uint SchemaVersion { get; init; } = CurrentSchemaVersion;

    [JsonPropertyName("document_revision")]
    public ulong DocumentRevision { get; init; }

    [JsonPropertyName("page_identity")]
    public uint PageIdentity { get; init; }

    [JsonPropertyName("page_number")]
    public int PageNumber { get; init; }

    [JsonPropertyName("dpi")]
    public uint Dpi { get; init; }

    [JsonPropertyName("page_box")]
    public RenderContractPageBox PageBox { get; init; } = RenderContractPageBox.Crop;

    [JsonPropertyName("transform")]
    public RenderContractDeviceMatrix Transform { get; init; } = RenderContractDeviceMatrix.Identity();

    [JsonPropertyName("clip")]
    public RenderContractDeviceClip? Clip { get; init; }

    [JsonPropertyName("width")]
    public uint Width { get; init; }

    [JsonPropertyName("height")]
    public uint Height { get; init; }

    [JsonPropertyName("stride")]
    public ulong Stride { get; init; }

    [JsonPropertyName("pixel_format")]
    public RenderContractPixelFormat PixelFormat { get; init; } = RenderContractPixelFormat.Rgba8;

    [JsonPropertyName("alpha_mode")]
    public RenderContractAlphaMode AlphaMode { get; init; } = RenderContractAlphaMode.Premultiplied;

    [JsonPropertyName("background")]
    public RenderContractColor Background { get; init; } = RenderContractColor.White();

    [JsonPropertyName("execution_mode")]
    public RenderContractExecutionMode ExecutionMode { get; init; } = RenderContractExecutionMode.Standard;

    [JsonPropertyName("backend")]
    public RenderContractBackendSelection Backend { get; init; } = RenderContractBackendSelection.StandardCpu;

    [JsonPropertyName("compositing")]
    public RenderContractCompositingPolicy Compositing { get; init; } =
        RenderContractCompositingPolicy.Compatibility;

    [JsonPropertyName("annotations")]
    public RenderContractAnnotationPolicy Annotations { get; init; } =
        RenderContractAnnotationPolicy.Include;

    [JsonPropertyName("forms")]
    public RenderContractFormPolicy Forms { get; init; } = RenderContractFormPolicy.Include;

    [JsonPropertyName("optional_content")]
    public string OptionalContent { get; init; } = "ocg:default";

    [JsonPropertyName("text_smoothing")]
    public RenderContractSmoothingPolicy TextSmoothing { get; init; } =
        RenderContractSmoothingPolicy.Antialiased;

    [JsonPropertyName("image_smoothing")]
    public RenderContractSmoothingPolicy ImageSmoothing { get; init; } =
        RenderContractSmoothingPolicy.Antialiased;

    [JsonPropertyName("path_smoothing")]
    public RenderContractSmoothingPolicy PathSmoothing { get; init; } =
        RenderContractSmoothingPolicy.Antialiased;

    [JsonPropertyName("subpixel_text")]
    public RenderContractSmoothingPolicy SubpixelText { get; init; } =
        RenderContractSmoothingPolicy.Disabled;

    [JsonPropertyName("grayscale")]
    public bool Grayscale { get; init; }

    [JsonPropertyName("color_scheme")]
    public RenderContractColorScheme ColorScheme { get; init; } = RenderContractColorScheme.Light;

    [JsonPropertyName("reverse_byte_order")]
    public bool ReverseByteOrder { get; init; }

    [JsonPropertyName("print_profile")]
    public RenderContractPrintProfile PrintProfile { get; init; } = RenderContractPrintProfile.Display;

    [JsonPropertyName("halftone")]
    public RenderContractHalftonePolicy Halftone { get; init; } = RenderContractHalftonePolicy.Disabled;

    [JsonPropertyName("overprint")]
    public RenderContractOverprintPolicy Overprint { get; init; } = RenderContractOverprintPolicy.Disabled;

    [JsonPropertyName("rendering_intent")]
    public RenderContractRenderingIntent RenderingIntent { get; init; } =
        RenderContractRenderingIntent.RelativeColorimetric;

    [JsonPropertyName("color_management")]
    public RenderContractColorManagementPolicy ColorManagement { get; init; } =
        RenderContractColorManagementPolicy.PortableQcms;

    [JsonPropertyName("exactness")]
    public RenderContractExactnessPolicy Exactness { get; init; } =
        RenderContractExactnessPolicy.Compatibility;

    [JsonPropertyName("determinism")]
    public RenderContractDeterminismPolicy Determinism { get; init; } =
        RenderContractDeterminismPolicy.Required;

    [JsonPropertyName("resource_budget")]
    public RenderContractResourceBudget ResourceBudget { get; init; } =
        RenderContractResourceBudget.Default();

    [JsonIgnore]
    public ulong SurfaceByteLength => checked(Stride * Height);

    public static RenderContract FromJson(string json)
    {
        ArgumentNullException.ThrowIfNull(json);
        var contract = JsonSerializer.Deserialize<RenderContract>(json, JsonOptions)
            ?? throw new ArgumentException("Render contract JSON did not contain an object.", nameof(json));
        contract.Validate();
        return contract;
    }

    public string ToJson()
    {
        Validate();
        return JsonSerializer.Serialize(this, JsonOptions);
    }

    public RenderContract WithSurface(
        uint width,
        uint height,
        RenderContractPixelFormat pixelFormat = RenderContractPixelFormat.Rgba8,
        RenderContractAlphaMode alphaMode = RenderContractAlphaMode.Premultiplied,
        ulong? stride = null,
        bool grayscale = false,
        bool reverseByteOrder = false)
    {
        var minimumStride = checked((ulong)width * BytesPerPixel(pixelFormat));
        var requestedStride = stride ?? minimumStride;
        if (requestedStride < minimumStride)
        {
            throw new ArgumentOutOfRangeException(
                nameof(stride),
                $"Render contract stride {requestedStride} is below the required {minimumStride} bytes.");
        }
        return this with
        {
            Width = width,
            Height = height,
            PixelFormat = pixelFormat,
            AlphaMode = alphaMode,
            Stride = requestedStride,
            Grayscale = grayscale,
            ReverseByteOrder = reverseByteOrder,
        };
    }

    public RenderContract WithClip(int x, int y, uint width, uint height)
    {
        if (width == 0 || height == 0)
        {
            throw new ArgumentOutOfRangeException(nameof(width), "Render contract clip must be non-empty.");
        }
        return this with { Clip = new RenderContractDeviceClip(x, y, width, height) };
    }

    public RenderContract WithoutClip()
    {
        return this with { Clip = null };
    }

    public RenderContract WithDeviceTransform(double a, double b, double c, double d, double e, double f)
    {
        return this with { Transform = RenderContractDeviceMatrix.FromF64(a, b, c, d, e, f) };
    }

    public RenderContract WithBackground(byte r, byte g, byte b, byte a = 255)
    {
        return this with { Background = new RenderContractColor(r, g, b, a) };
    }

    public RenderContract WithPageBox(RenderContractPageBox pageBox)
    {
        return this with { PageBox = RequireDefined(pageBox, nameof(pageBox)) };
    }

    public RenderContract WithExecutionMode(RenderContractExecutionMode executionMode)
    {
        return this with { ExecutionMode = RequireDefined(executionMode, nameof(executionMode)) };
    }

    public RenderContract WithBackend(RenderContractBackendSelection backend)
    {
        return this with { Backend = RequireDefined(backend, nameof(backend)) };
    }

    public RenderContract WithCompositing(RenderContractCompositingPolicy compositing)
    {
        return this with { Compositing = RequireDefined(compositing, nameof(compositing)) };
    }

    public RenderContract WithAnnotations(RenderContractAnnotationPolicy annotations)
    {
        return this with { Annotations = RequireDefined(annotations, nameof(annotations)) };
    }

    public RenderContract WithForms(RenderContractFormPolicy forms)
    {
        return this with { Forms = RequireDefined(forms, nameof(forms)) };
    }

    public RenderContract WithOptionalContent(string optionalContent)
    {
        if (string.IsNullOrWhiteSpace(optionalContent))
        {
            throw new ArgumentException("Render contract optional-content identity must be present.", nameof(optionalContent));
        }
        return this with { OptionalContent = optionalContent };
    }

    public RenderContract WithSmoothing(RenderContractSmoothingPolicy smoothing)
    {
        var checkedSmoothing = RequireDefined(smoothing, nameof(smoothing));
        return this with
        {
            TextSmoothing = checkedSmoothing,
            ImageSmoothing = checkedSmoothing,
            PathSmoothing = checkedSmoothing,
        };
    }

    public RenderContract WithTextSmoothing(RenderContractSmoothingPolicy textSmoothing)
    {
        return this with { TextSmoothing = RequireDefined(textSmoothing, nameof(textSmoothing)) };
    }

    public RenderContract WithImageSmoothing(RenderContractSmoothingPolicy imageSmoothing)
    {
        return this with { ImageSmoothing = RequireDefined(imageSmoothing, nameof(imageSmoothing)) };
    }

    public RenderContract WithPathSmoothing(RenderContractSmoothingPolicy pathSmoothing)
    {
        return this with { PathSmoothing = RequireDefined(pathSmoothing, nameof(pathSmoothing)) };
    }

    public RenderContract WithSubpixelText(RenderContractSmoothingPolicy subpixelText)
    {
        return this with { SubpixelText = RequireDefined(subpixelText, nameof(subpixelText)) };
    }

    public RenderContract WithColorScheme(RenderContractColorScheme colorScheme)
    {
        return this with { ColorScheme = RequireDefined(colorScheme, nameof(colorScheme)) };
    }

    public RenderContract WithPrintProfile(RenderContractPrintProfile printProfile)
    {
        return this with { PrintProfile = RequireDefined(printProfile, nameof(printProfile)) };
    }

    public RenderContract WithHalftone(RenderContractHalftonePolicy halftone)
    {
        return this with { Halftone = RequireDefined(halftone, nameof(halftone)) };
    }

    public RenderContract WithOverprint(RenderContractOverprintPolicy overprint)
    {
        return this with { Overprint = RequireDefined(overprint, nameof(overprint)) };
    }

    public RenderContract WithRenderingIntent(RenderContractRenderingIntent renderingIntent)
    {
        return this with { RenderingIntent = RequireDefined(renderingIntent, nameof(renderingIntent)) };
    }

    public RenderContract WithColorManagement(RenderContractColorManagementPolicy colorManagement)
    {
        return this with { ColorManagement = RequireDefined(colorManagement, nameof(colorManagement)) };
    }

    public RenderContract WithExactness(RenderContractExactnessPolicy exactness)
    {
        return this with { Exactness = RequireDefined(exactness, nameof(exactness)) };
    }

    public RenderContract WithDeterminism(RenderContractDeterminismPolicy determinism)
    {
        return this with { Determinism = RequireDefined(determinism, nameof(determinism)) };
    }

    public RenderContract WithResourceBudget(
        ulong? maxPixels = null,
        ulong? maxDecodedBytes = null,
        ulong? maxTemporaryBytes = null,
        ulong? maxCacheBytes = null)
    {
        var budget = ResourceBudget ?? RenderContractResourceBudget.Default();
        return this with
        {
            ResourceBudget = budget with
            {
                MaxPixels = maxPixels ?? budget.MaxPixels,
                MaxDecodedBytes = maxDecodedBytes ?? budget.MaxDecodedBytes,
                MaxTemporaryBytes = maxTemporaryBytes ?? budget.MaxTemporaryBytes,
                MaxCacheBytes = maxCacheBytes ?? budget.MaxCacheBytes,
            },
        };
    }

    public void Validate()
    {
        if (SchemaVersion != CurrentSchemaVersion)
        {
            throw new InvalidOperationException(
                $"Render contract schema {SchemaVersion} is unsupported; expected {CurrentSchemaVersion}.");
        }
        if (PageNumber < 1)
        {
            throw new InvalidOperationException("Render contract page number must be 1-based.");
        }
        if (Width == 0 || Height == 0)
        {
            throw new InvalidOperationException("Render contract output width and height must be non-zero.");
        }
        if (Transform is null)
        {
            throw new InvalidOperationException("Render contract device transform must be present.");
        }
        Transform.Validate();
        if (Background is null)
        {
            throw new InvalidOperationException("Render contract background must be present.");
        }
        Background.Validate();
        if (string.IsNullOrWhiteSpace(OptionalContent))
        {
            throw new InvalidOperationException("Render contract optional-content identity must be present.");
        }
        if (ResourceBudget is null)
        {
            throw new InvalidOperationException("Render contract resource budget must be present.");
        }
        var minimumStride = checked((ulong)Width * BytesPerPixel(PixelFormat));
        if (Stride < minimumStride)
        {
            throw new InvalidOperationException(
                $"Render contract stride {Stride} is below the required {minimumStride} bytes.");
        }
        if (checked((ulong)Width * Height) > ResourceBudget.MaxPixels)
        {
            throw new InvalidOperationException(
                $"Render contract requests {checked((ulong)Width * Height)} pixels, exceeding budget {ResourceBudget.MaxPixels}.");
        }
        if (Clip is { Width: 0 } or { Height: 0 })
        {
            throw new InvalidOperationException("Render contract clip must have non-zero dimensions.");
        }
        ValidateEnum(PageBox, nameof(PageBox));
        ValidateEnum(PixelFormat, nameof(PixelFormat));
        ValidateEnum(AlphaMode, nameof(AlphaMode));
        ValidateEnum(ExecutionMode, nameof(ExecutionMode));
        ValidateEnum(Backend, nameof(Backend));
        ValidateEnum(Compositing, nameof(Compositing));
        ValidateEnum(Annotations, nameof(Annotations));
        ValidateEnum(Forms, nameof(Forms));
        ValidateEnum(TextSmoothing, nameof(TextSmoothing));
        ValidateEnum(ImageSmoothing, nameof(ImageSmoothing));
        ValidateEnum(PathSmoothing, nameof(PathSmoothing));
        ValidateEnum(SubpixelText, nameof(SubpixelText));
        ValidateEnum(ColorScheme, nameof(ColorScheme));
        ValidateEnum(PrintProfile, nameof(PrintProfile));
        ValidateEnum(Halftone, nameof(Halftone));
        ValidateEnum(Overprint, nameof(Overprint));
        ValidateEnum(RenderingIntent, nameof(RenderingIntent));
        ValidateEnum(ColorManagement, nameof(ColorManagement));
        ValidateEnum(Exactness, nameof(Exactness));
        ValidateEnum(Determinism, nameof(Determinism));
    }

    public static uint BytesPerPixel(RenderContractPixelFormat pixelFormat)
    {
        return pixelFormat switch
        {
            RenderContractPixelFormat.Rgba8 or RenderContractPixelFormat.Bgra8 => 4,
            RenderContractPixelFormat.Rgb8 or RenderContractPixelFormat.Bgr8 => 3,
            RenderContractPixelFormat.Gray8 => 1,
            _ => throw new ArgumentOutOfRangeException(nameof(pixelFormat), pixelFormat, null),
        };
    }

    private static T RequireDefined<T>(T value, string name)
        where T : struct, Enum
    {
        if (!Enum.IsDefined(value))
        {
            throw new ArgumentOutOfRangeException(name, value, $"Render contract {name} value is unsupported.");
        }
        return value;
    }

    private static void ValidateEnum<T>(T value, string name)
        where T : struct, Enum
    {
        if (!Enum.IsDefined(value))
        {
            throw new InvalidOperationException($"Render contract {name} value is unsupported.");
        }
    }
}

public sealed record RenderContractDeviceMatrix
{
    [JsonPropertyName("values")]
    public ulong[] Values { get; init; } = IdentityValues();

    public static RenderContractDeviceMatrix Identity()
    {
        return new RenderContractDeviceMatrix { Values = IdentityValues() };
    }

    public static RenderContractDeviceMatrix FromF64(double a, double b, double c, double d, double e, double f)
    {
        return new RenderContractDeviceMatrix
        {
            Values =
            [
                BitConverter.DoubleToUInt64Bits(a),
                BitConverter.DoubleToUInt64Bits(b),
                BitConverter.DoubleToUInt64Bits(c),
                BitConverter.DoubleToUInt64Bits(d),
                BitConverter.DoubleToUInt64Bits(e),
                BitConverter.DoubleToUInt64Bits(f),
            ],
        };
    }

    public double[] ToF64()
    {
        Validate();
        return Values.Select(BitConverter.UInt64BitsToDouble).ToArray();
    }

    public void Validate()
    {
        if (Values is not { Length: 6 })
        {
            throw new InvalidOperationException("Render contract device transform must contain six values.");
        }
        var decoded = Values.Select(BitConverter.UInt64BitsToDouble).ToArray();
        if (decoded.Any(value => !double.IsFinite(value)))
        {
            throw new InvalidOperationException("Render contract transform must contain only finite values.");
        }
        var determinant = decoded[0] * decoded[3] - decoded[1] * decoded[2];
        if (Math.Abs(determinant) < 1e-10)
        {
            throw new InvalidOperationException("Render contract transform must be invertible.");
        }
    }

    private static ulong[] IdentityValues()
    {
        return
        [
            BitConverter.DoubleToUInt64Bits(1.0),
            BitConverter.DoubleToUInt64Bits(0.0),
            BitConverter.DoubleToUInt64Bits(0.0),
            BitConverter.DoubleToUInt64Bits(1.0),
            BitConverter.DoubleToUInt64Bits(0.0),
            BitConverter.DoubleToUInt64Bits(0.0),
        ];
    }
}

public sealed record RenderContractColor(
    [property: JsonPropertyName("r")] byte R,
    [property: JsonPropertyName("g")] byte G,
    [property: JsonPropertyName("b")] byte B,
    [property: JsonPropertyName("a")] byte A)
{
    public static RenderContractColor White()
    {
        return new RenderContractColor(255, 255, 255, 255);
    }

    public void Validate()
    {
    }
}

public sealed record RenderContractDeviceClip(
    [property: JsonPropertyName("x")] int X,
    [property: JsonPropertyName("y")] int Y,
    [property: JsonPropertyName("width")] uint Width,
    [property: JsonPropertyName("height")] uint Height);

public sealed record RenderContractResourceBudget
{
    [JsonPropertyName("max_pixels")]
    public ulong MaxPixels { get; init; } = 100_000_000;

    [JsonPropertyName("max_decoded_bytes")]
    public ulong MaxDecodedBytes { get; init; } = 512UL * 1024 * 1024;

    [JsonPropertyName("max_temporary_bytes")]
    public ulong MaxTemporaryBytes { get; init; } = 256UL * 1024 * 1024;

    [JsonPropertyName("max_cache_bytes")]
    public ulong MaxCacheBytes { get; init; } = 256UL * 1024 * 1024;

    public static RenderContractResourceBudget Default()
    {
        return new RenderContractResourceBudget();
    }
}

public enum RenderContractPageBox
{
    Media,
    Crop,
    Bleed,
    Trim,
    Art,
}

public enum RenderContractPixelFormat
{
    Rgba8,
    Bgra8,
    Rgb8,
    Bgr8,
    Gray8,
}

public enum RenderContractAlphaMode
{
    Premultiplied,
    Straight,
    Opaque,
}

public enum RenderContractExecutionMode
{
    Standard,
    Research,
}

public enum RenderContractBackendSelection
{
    ScalarReference,
    StandardCpu,
    ResearchHybrid,
}

public enum RenderContractSmoothingPolicy
{
    Disabled,
    Antialiased,
    Subpixel,
}

public enum RenderContractAnnotationPolicy
{
    Include,
    Exclude,
}

public enum RenderContractFormPolicy
{
    Include,
    Exclude,
}

public enum RenderContractColorScheme
{
    Light,
    Dark,
    ForcedMonochrome,
}

public enum RenderContractPrintProfile
{
    Display,
    Print,
    Proof,
}

public enum RenderContractHalftonePolicy
{
    Disabled,
    Screen,
}

public enum RenderContractOverprintPolicy
{
    Disabled,
    Preview,
    PreserveSeparations,
}

public enum RenderContractRenderingIntent
{
    RelativeColorimetric,
    AbsoluteColorimetric,
    Perceptual,
    Saturation,
}

public enum RenderContractColorManagementPolicy
{
    PortableQcms,
    NativeLittleCms,
    DeterministicFallback,
}

public enum RenderContractExactnessPolicy
{
    Compatibility,
    HighQualityExact,
}

public enum RenderContractDeterminismPolicy
{
    Required,
    BestEffortResearch,
}

public enum RenderContractCompositingPolicy
{
    Compatibility,
    HighQuality,
}
