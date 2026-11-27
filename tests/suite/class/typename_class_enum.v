// $typename of classes and enums (IEEE 1800-2017 20.6.1): a user-defined type's name
// after its package or module (e, f); an enum lists its constants with their values as
// sized decimal literals (signed for a signed base type); a typedef of a type names that
// type.
package A;
  class C;
  endclass
  typedef enum {X, Y, Z = 99} e_t;
  typedef enum bit [1:0] {P, Q = 3} b_t;
  typedef e_t e2_t;
endpackage
class U;
endclass

module top;
  class M;
  endclass
  typedef enum logic [3:0] {K = 4'd5} m_t;
  integer errors = 0;
  task check(input integer line, input string got, input string want);
    if (got != want) begin
      $display("FAILED line %0d: got '%s' want '%s'", line, got, want);
      errors = errors + 1;
    end
  endtask
  A::C c;
  A::e_t v;

  initial begin
    check(`__LINE__, $typename(A::C), "A::C");
    check(`__LINE__, $typename(c), "A::C");
    check(`__LINE__, $typename(U), "$unit::U");
    check(`__LINE__, $typename(M), "top::M");
    check(`__LINE__, $typename(A::e_t), "enum{X=32'sd0,Y=32'sd1,Z=32'sd99}A::e_t");
    check(`__LINE__, $typename(v), "enum{X=32'sd0,Y=32'sd1,Z=32'sd99}A::e_t");
    check(`__LINE__, $typename(A::e2_t), "enum{X=32'sd0,Y=32'sd1,Z=32'sd99}A::e_t");
    check(`__LINE__, $typename(A::b_t), "enum{P=2'd0,Q=2'd3}A::b_t");
    check(`__LINE__, $typename(m_t), "enum{K=4'd5}top::m_t");
    if (errors == 0) $display("PASSED");
  end
endmodule
