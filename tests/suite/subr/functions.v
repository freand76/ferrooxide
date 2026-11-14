// Functions (IEEE 1800 13.4): return by name and by `return`, recursion with automatic
// storage, static storage kept between calls, output and inout arguments, default argument
// values, void functions, constant functions in declarations, signed and ranged results
module t;
  integer errors = 0;
  function automatic integer fact(input integer n);
    if (n <= 1) return 1;
    return n * fact(n - 1);
  endfunction
  function integer counter(input integer inc);   // static: `count` persists
    integer count;
    if (inc == 0) count = 0;
    count = count + inc;
    counter = count;
  endfunction
  function automatic void split(input [7:0] v, output [3:0] hi, output [3:0] lo);
    hi = v[7:4];
    lo = v[3:0];
  endfunction
  function automatic integer add_to(inout integer acc, input integer v = 1);
    acc = acc + v;
    return acc;
  endfunction
  function integer clog2c(input integer v);       // a constant function
    clog2c = 0;
    while ((1 << clog2c) < v) clog2c = clog2c + 1;
  endfunction
  function signed [7:0] neg(input [7:0] v);
    neg = -v;
  endfunction
  function automatic integer plus(input integer i, input integer j = da + db);
    return i + j;
  endfunction
  integer da = 1, db = 2;
  wire [31:0] cw = plus(10);           // follows da and db through the default value
  localparam W = clog2c(100);
  reg [W-1:0] sized;
  integer a, r;
  reg [3:0] h, l;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    check(`__LINE__, fact(5), 120);
    check(`__LINE__, fact(10), 3628800);
    r = counter(0);
    r = counter(3);
    r = counter(4);
    check(`__LINE__, r, 7);
    split(8'hc5, h, l);
    check(`__LINE__, {h, l}, 8'hc5);
    a = 10;
    r = add_to(a, 5);
    check(`__LINE__, a, 15);
    r = add_to(a);                                 // default v = 1
    check(`__LINE__, a, 16);
    check(`__LINE__, W, 7);
    check(`__LINE__, $bits(sized), 7);
    a = neg(8'd3);                                 // a signed result is sign-extended
    check(`__LINE__, a, -3);
    #1 check(`__LINE__, cw, 13);
    db = 5;
    #1 check(`__LINE__, cw, 16);
    if (errors == 0) $display("PASSED");
  end
endmodule
