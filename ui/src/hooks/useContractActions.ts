import { useCallback } from 'react';
import type { Dispatch, SetStateAction } from 'react';
import {
  approveContractCandidate,
  createContractCandidate,
  supersedeRegressionContract,
  updateContractCandidate,
  type ContractMutationResponse,
} from '../lib/api';
import type { BootstrapData, RegressionCandidateDraftV1 } from '../types';
import type { PerformMutation } from './useMutationRunner';

interface ContractActionsOptions {
  data: BootstrapData | null;
  setData: Dispatch<SetStateAction<BootstrapData | null>>;
  performMutation: PerformMutation;
}

export function useContractActions({ data, setData, performMutation }: ContractActionsOptions) {
  const createCandidate = useCallback(async (draft: RegressionCandidateDraftV1) => {
    const result = await performMutation(() => createContractCandidate(draft));
    if (!result) return false;
    setData((current) => current ? mergeContractMutation(current, result) : current);
    return true;
  }, [performMutation, setData]);

  const updateCandidate = useCallback(async (id: string, draft: RegressionCandidateDraftV1) => {
    const previous = data?.contractCandidates.find((candidate) => candidate.id === id);
    setData((current) => current ? {
      ...current,
      contractCandidates: current.contractCandidates.map((candidate) => candidate.id === id ? { ...candidate, ...draft } : candidate),
    } : current);
    const result = await performMutation(() => updateContractCandidate(id, draft));
    if (!result) {
      if (previous) {
        setData((current) => current ? {
          ...current,
          contractCandidates: current.contractCandidates.map((candidate) => candidate.id === id ? previous : candidate),
        } : current);
      }
      return false;
    }
    setData((current) => current ? mergeContractMutation(current, result) : current);
    return true;
  }, [data, performMutation, setData]);

  const approveCandidate = useCallback(async (id: string) => {
    const result = await performMutation(() => approveContractCandidate(id));
    if (!result) return false;
    setData((current) => current ? mergeContractMutation({
      ...current,
      contractCandidates: current.contractCandidates.filter((candidate) => candidate.id !== id),
    }, result) : current);
    return true;
  }, [performMutation, setData]);

  const supersedeContract = useCallback(async (id: string, supersededBy: string) => {
    const previous = data?.contracts.find((contract) => contract.id === id);
    setData((current) => current ? {
      ...current,
      contracts: current.contracts.map((contract) => contract.id === id ? { ...contract, status: 'superseded', supersededBy } : contract),
    } : current);
    const result = await performMutation(() => supersedeRegressionContract(id, supersededBy));
    if (!result) {
      if (previous) {
        setData((current) => current ? {
          ...current,
          contracts: current.contracts.map((contract) => contract.id === id ? previous : contract),
        } : current);
      }
      return false;
    }
    setData((current) => current ? mergeContractMutation(current, result) : current);
    return true;
  }, [data, performMutation, setData]);

  return { createCandidate, updateCandidate, approveCandidate, supersedeContract };
}

function mergeContractMutation(current: BootstrapData, response: ContractMutationResponse): BootstrapData {
  let contracts = response.contracts ?? current.contracts;
  let contractCandidates = response.contractCandidates ?? current.contractCandidates;
  if (!response.contracts && response.contract) {
    const exists = contracts.some((contract) => contract.id === response.contract?.id);
    contracts = exists
      ? contracts.map((contract) => contract.id === response.contract?.id ? response.contract! : contract)
      : [...contracts, response.contract];
  }
  if (!response.contractCandidates && response.candidate) {
    const exists = contractCandidates.some((candidate) => candidate.id === response.candidate?.id);
    contractCandidates = exists
      ? contractCandidates.map((candidate) => candidate.id === response.candidate?.id ? response.candidate! : candidate)
      : [...contractCandidates, response.candidate];
  }
  return {
    ...current,
    contracts,
    contractCandidates,
    contractEvaluation: response.contractEvaluation === undefined ? current.contractEvaluation : response.contractEvaluation,
    contractEvaluations: current.contractEvaluations,
  };
}
